# btw crowd and mood: the room as an input

*Design only. Nothing here is implemented.* Branch `feat/btw-web` (this file only). Written 2026-11-18 for Matt.

**The ask.** While a btw program plays, the listener, or a DJ reporting the room, can say how it feels with one tap.
The program can read that (`crowd`, `mood`) at its evolve generation boundaries. Every value it reads is logged, so a
night replays exactly from the program's CID plus the input log. A side pane turns recent picks into suggested
instructions for compose or edit, which come back as a diff the user previews and accepts as a fork. Claude can read
and set the vibe over MCP. Picks are anonymous and local until the user saves or shares.

## Contents

0. [The design in one page](#0-the-design-in-one-page)
1. [Problem and inspiration](#1-problem-and-inspiration)
2. [The language (a proposal)](#2-the-language-a-proposal)
3. [The performed tier: data model](#3-the-performed-tier-data-model)
4. [Worked example](#4-worked-example)
5. [The vibe modal](#5-the-vibe-modal)
6. [The guide pane](#6-the-guide-pane)
7. [HTTP and MCP surface](#7-http-and-mcp-surface)
8. [Trust, abuse, rooms and privacy](#8-trust-abuse-rooms-and-privacy)
9. [v1 scope, build order, risks](#9-v1-scope-build-order-risks)
10. [Test plan](#10-test-plan)
11. [Open questions, with my recommendation](#11-open-questions-with-my-recommendation)
12. [Appendix: what changes in existing docs](#12-appendix-what-changes-in-existing-docs)

---

## 0. The design in one page

btw has two tiers today: the **authored** one (the program text, its CID) and the **derived** one (evolve generations,
each a function of the text; BTW.md "Evolving"). This adds a third, the **performed** tier (Matt's word): what a human
or an agent told the program while it played. It is a *second input*, never part of the text.

| # | Decision | § |
|---|---|---|
| 1 | A **performance** is `(program CID, input log)`. The program CID never changes with what was performed; the log has its own CID. Same pair, same night, generation CID for CID. | 2.4, 3 |
| 2 | A **closed vocabulary** `btw.vibe/1`: six crowd words, six solo words, `neutral`, three intensities. Numbers come from one integer table (hundredths), so every host computes the same values. | 2.1 |
| 3 | **One new read form**, `<crowd.calm>`, and **one extended guard**, `when mood == restless and gen >= 2`. Legal only in `evolve` rules; only `thin` takes a read number. The rest of the language is unchanged. | 2.2 |
| 4 | **Read once per generation, in the bar before it starts** (the runtime's own apply window), never mid-bar. All rules in a generation see one snapshot. | 2.3 |
| 5 | **The log is the only door.** The UI appends to the log; the evolver sees a frozen view of the log. Live and replay run the same code, so the log is complete by construction. | 2.4 |
| 6 | **Neutral** (the middle of every scale) is the default, and a pick expires to neutral after 64 bars. A program with no input is the program it always was. | 2.3 |
| 7 | `check` requires a **bound** for an evolve that reads input, reports the **read alphabet**, and gives tail and period for neutral and each constant input. It cannot prove an orbit for an arbitrary log, and says so. | 2.5 |
| 8 | A **performance object** has its own CID, link form and rights (most restrictive of its own and the program's). **Private by default**, unlike a program, because a mood log is personal. | 3 |
| 9 | **Vibe modal**: one tap, labelled buttons, intensity optional, local. A prompt chip at section changes appears **only when the program reads input or the guide is open**, and can be switched off. | 5 |
| 10 | **Guide pane**: a *deterministic rule table* (no model) turns recent picks into 2 or 3 suggested instructions. Sending one is the debugger's flow: context bundle, propose, preview, accept as a fork. | 6 |
| 11 | **MCP**: `crowd_get`, `crowd_set`, `mood_get`, `mood_set`; bearer token plus a per-session consent toggle on the page; sets are logged `origin: mcp`. | 7 |
| 12 | **v1 = single user and a DJ.** No rooms, no sensors, no camera or mic. | 8, 9 |

### What I read, and how sure I am

Read in full: `docs/BTW.md` (the language claims are checked against it); `BTW-DEBUGGER.md` on `origin/design/btw-debugger`
(its vocabulary is reused and dependencies on it are named); `btwsite/DESIGN.md` and `internal/btwsite/api/api.go`.
**Not read for this document: the `djlive` source** (`runtime.rs`, `compile.rs`, `evolve.rs`). The runtime facts used (the 0.75
apply window, the bar-ahead load, a chain computed at compile time) are BTW.md's and the debugger design's account, and spikes
S1 to S3 (§9) verify them first. *Walkaway* (Doctorow, 2017) is from memory, not re-read: only its premise is used.

---

## 1. Problem and inspiration

**Problem.** A btw program is total and deterministic: the same text is the same music, forever. `evolve` gives it
variation over time, but the variation is blind. It cannot know that the room has stopped dancing, that the listener
is restless, or that everyone is waiting for a drop. A human DJ reads the room and changes the set; a btw user must
stop, edit the text and re-evaluate. We want the room to be a first-class input to the program without giving up what
the language is built on: same text, same CID; every program ends; a recorded night replays exactly. And we want the
human's reaction helped: "the room has been drifting for 16 bars" should become a suggested, checkable, previewable edit.

**Inspiration.** As I remember *Walkaway*, the party is scored by music that mutates itself in response to the people
dancing to it: not a playlist, not a human at a desk, but a process the crowd steers. btw already has the mutation half
(`evolve`) and a way to make the steering *auditable*: a program is a pure function of its text and a bar number, so the
crowd's influence reduces to a short log of tagged values, and the night is `program + log`. The book's party cannot be
repeated; ours can be replayed, forked and shared, and still steered live. The second half runs the other way: the signal
that steers the music can also steer the *author*, by suggesting what to change in the text.

---

## 2. The language (a proposal)

**Status.** Today `evolve` rules take literal numbers and `gen` guards only (BTW.md "Grammar: the additions"), and
nothing reads from outside the program. **Everything in this section is a proposal**, kept to the minimum that makes the
feature real: two reserved words, one argument form, one extended guard. Parsing, canonical form, `let`, `def`, sections,
`arrange` and the generation CIDs of any program that does not mention `crowd` or `mood` are unchanged (a test pins the
corpus CIDs). `crowd.calm` is written in the text, so it is part of the program CID; **what was performed is not.**

### 2.1 The vocabulary `btw.vibe/1`

Closed and versioned. A new word is `btw.vibe/2`, never a silent edit; a log names its vocabulary.

```text
crowd words : hyped  dancing  chilling  drifting  leaving  wants_drop           (and neutral)
solo words  : energised  focused  mellow  sad  restless  bored                  (and neutral)
intensity   : 1 "a little"   2 "normal" (default)   3 "a lot"
numbers     : integers in hundredths, 0..100, read as 0..1
scaling     : x_i = clamp(50 + (x_2 - 50) * i / 2, 0, 100)    exact: every x_2 - 50 below is a multiple of 10
neutral     : energy 50, valence 50      calm = 100 - energy
```

| source | word | energy at 2 (at 1, at 3) | valence at 2 | meaning |
|---|---|---|---|---|
| crowd | `hyped` | 90 (70, 100) | - | the room is peaking |
| crowd | `dancing` | 70 (60, 80) | - | steady and moving |
| crowd | `chilling` | 40 (45, 35) | - | relaxed, talking, swaying |
| crowd | `drifting` | 20 (35, 5) | - | attention is slipping away |
| crowd | `leaving` | 10 (30, 0) | - | people are going |
| crowd | `wants_drop` | 60 (55, 65) | - | a request, not a level: they want a release |
| solo | `energised` | 90 (70, 100) | 80 | |
| solo | `focused` | 60 (55, 65) | 60 | steady attention, wants no surprises |
| solo | `mellow` | 30 (40, 20) | 70 | |
| solo | `sad` | 20 (35, 5) | 20 | |
| solo | `restless` | 70 (60, 80) | 30 | wants movement, cannot settle |
| solo | `bored` | 10 (30, 0) | 40 | |

The full 12 x 3 table is a generated golden (`vocab/btw.vibe.1.json`) shared by the Rust, the JS and the tests. A crowd
has no valence (`crowd.valence` is an error that says so). Matt's brief used "restless" as a crowd example; it is a solo
word here and `drifting` is the nearest crowd word (question 2).

### 2.2 Syntax

In the style of BTW.md; only lines marked `new` or `changed` differ from the current grammar.

```ebnf
(* ---- proposal: reading the room. In evolve rules only. ---- *)
rule        = ( role | "deck" deck ) ":" op { "|" op } [ guard ] ;            (* changed: guard *)
op          = ( "rotate" | "transpose" | "octave" | "invert" | "reverse" | "thin" )
              { number | "<" gen_arg { gen_arg } ">" } [ "seed=" int ] ;      (* changed: gen_arg *)
gen_arg     = number | input_num ;                                            (* new *)
guard       = "when" cond { "and" cond } ;                                    (* new: and *)
cond        = "gen" [ "%" int ] cmp int                                       (* as before *)
            | input_num cmp number                                            (* new *)
            | input_word ( "==" | "!=" ) WORD ;                               (* new *)
input_num   = "crowd" "." ( "energy" | "calm" )                               (* new: a number 0..1 *)
            | "mood" "." ( "energy" | "calm" | "valence" ) ;
input_word  = "crowd" "." "mood" | "mood" ;                                   (* new: a word *)
(* WORD: a word of that source's vocabulary, or "neutral". `crowd` and `mood` become reserved words. *)
```

- A reference **in angle brackets** is a per-generation value, like `<0 5 7 0>`: `thin <crowd.calm>` is "thin by the room's calm,
  read afresh each generation"; `<0.1 crowd.calm 0.3>` reads only in generation 2. A bare `thin crowd.calm` is an error that
  suggests the brackets: they mark a value that is performed, not written.
- Only `thin` (it takes a share) accepts an input number; `transpose <crowd.energy>` is an error ("`transpose` takes whole
  numbers; `crowd.energy` is 0 to 1"). Words are typed by source: `crowd.mood == restless` fails with "`restless` is a solo word:
  write `mood == restless`; the crowd's words are hyped, dancing, chilling, drifting, leaving, wants_drop, neutral".
- `calm` is sugar for `1 - energy`, because `thin` removes (question 1). Percent works in guards (`crowd.energy < 30%`).
- Inputs are **not** readable in control lines, `let`, or generation 0 ("the parts as written"), so the first reaction is
  generation 1; use a shorter `every` for a quicker one (control lines: question 7). `crowd` and `mood` become reserved words,
  so `let mood = 8` becomes an error: grep the corpus before landing (step 0, §9).

### 2.3 What "read" means

The runtime already stages work a bar ahead: an evaluation is applied "in the first three beats of a bar" (BTW.md
"Evaluating and hot reload"; `APPLY_BY = 0.75`), and a generation is compiled as a segment swapped in on its bar line, so
its compile happens in the bar before. The read goes there.

```text
read point of bar k   = beat 3.0 of program bar k                   (0.75 of the bar: the apply window's edge)
stamp(bar k, beat f)  = k if f < 3.0 else k + 1                     (the bar whose read point a pick is first before)
generation n, starting on bar b_n, reads  snapshot_at(log, b_n)  at the read point of bar b_n - 1

snapshot_at(log, b), for each source s in {crowd, mood}:
    e = the last entry of s in log with t_bar <= b - 1              (array order breaks ties; the later wins)
    s = neutral  if there is no e, or (b - 1) - e.t_bar >= EXPIRY_BARS (64);  else s = e
```

- **`t_bar` is the played-bar count of the program** (the `rel` the debugger's `frame_at` uses), counting forward only.
  The stamp is one Rust function (`djlive::input::stamp`) used by every host, so JS, desktop and the renderer cannot
  disagree about which bar a pick belongs to.
- **Never mid-bar.** A pick at beat 3.4 of bar 10 is stamped 11 and is seen by a generation starting on bar 12 or later,
  not the one starting on bar 11. The UI says which bar a pick will first be read on.
- **One snapshot per generation:** every rule of the block in generation n sees the same `snapshot_at(log, b_n)`.
- **Neutral and expiry.** No pick, or one 64 or more bars old, reads as `neutral` (`crowd.mood == neutral`, energy and
  calm 0.5). A room goes stale, and a mood from two hours ago must not steer the music. "Clear" in the UI logs an
  explicit `neutral`.
- **Hold or seek** (debugger) does not advance the program clock; picks made while held take the held bar and collapse
  to the last one (§3.1).

### 2.4 Totality and determinism

**T1, total.** An input reference is a constant lookup in a finite table at a fixed point; nothing recurses, loops or needs an
unbounded count. An `evolve` that reads input **must have a bound** (`stop when gen == N`, `for N gens`, or a section, whose
length is the bound); with no sections and no `stop` it is an error ("the orbit of an evolve that reads the room depends on the
log; add `stop when gen == N`"). Generations stay at most 256.

**T2, the program CID does not see the input.** The text holds `<crowd.calm>` and nothing about its value; the canonical text,
program CID, `fmt` and generation 0 are as before. The input is an explicit second argument: `perform(program, log)`.

**D1, deterministic.** Generation n is `f_n(generation n-1, snapshot_at(log, b_n))`, and `snapshot_at` is pure in `(log, b_n)`:
no clock, no UI state, no randomness. `thin`'s hash is of (seed, generation, bar, step, note), none of which is the input. So
`(program CID, log CID)` fixes every generation, hence every net CID, hence (same nets, same render) the audio.

**D2, the log is complete:** everything the output depends on outside the program passes through it. By structure, not care.
The evolver takes input only as `&dyn InputView`, built only by `InputLog::view()` (a frozen copy). The UI never hands its
current value to the runtime: it calls `Live::push_input`, which stamps the entry and **appends it to the log**, and the
evolver reads the log, so live and replay are one code path. The **record-replay test** (§10) runs live with random picks,
saves, replays from the object alone and demands the same generation CIDs and command log; **witnesses** (§4) make a replay on
a different build say *where* it differs rather than drift silently.

**The cost.** `btw generation {gen: N}` today answers "from generation 0 and the rule alone", jumping over the orbit. With
input, N depends on the log: `generation(program, log, N)` costs N generations (at most 256). With no log the shortcut stands.

### 2.5 What `check` reports, and what it cannot prove

```text
evolve in groove: reads crowd.calm, crowd.mood, mood
  bound: 3 generations (stop when gen == 3); read at the read points of bars 7, 15, 23
  read alphabet: crowd 19 x mood 7 = 133 symbols a generation; at most 133^3 = 2,352,637 distinct logs matter
  no input (neutral): hihat thins 0.5 a generation; arp and bass unchanged; ends at generation 3
  constant crowd=hyped/3: arp rotate 2 | transpose 7 from generation 1; hihat unthinned (calm 0)
  not proved: the orbit under an arbitrary log (the log can steer to any state its alphabet reaches; the bound is the guarantee)
```

- **The alphabet** is the distinct tuples of the properties the program *actually reads*, per source: `crowd.calm` and
  `crowd.mood` give the 18 (word, intensity) picks plus neutral, 19; `mood` alone gives 7. It is bounded by the vocabulary
  (at most 19 per source, 361 for both).
- **Proved:** it ends (T1), the bound, determinism, every read typed and in range, no rule dead by type, a well-defined
  *neutral performance*, and for each constant symbol (up to 64, then "64 of 361") the existing tail and period, since a
  constant symbol makes the rules input-free.
- **Not proved:** the orbit of an arbitrary log. A changing log can follow any path its alphabet allows, so `check` reports the
  bound as the guarantee and the per-symbol orbits as *examples*; it can warn of a degenerate constant case ("`hihat` has no
  hits from generation 2 under crowd=leaving/3"). The future log is unknown, so the debugger's footprint of an edit is computed
  with inputs **held at their standing values** and labelled so (§6.6).

### 2.6 Hosts

| host | what happens |
|---|---|
| **Browser** (`djweb`, `BtwRun`) | `BtwRun.input(source, key, intensity, origin)` stamps from its own clock and appends; `input_log()` and `inputs()` return JSON. The page never stamps in JS. |
| **Desktop** (`djapp`) | the same `Live`; picks come from the btw window or the `crowd` / `mood` registry nouns (§7.2); `btw get` carries `inputs`. |
| **Offline renderer** (`btwd render`, `djctl btw render`, `btw_render`) | takes the whole log up front (`--perf FILE`, a performance or a bare log), compiles every generation and renders; the cache key gains the performance CID; no log renders the neutral performance. |

**The change this forces (the largest in the design).** Today `eval` answers the whole `lineage`, and each generation is a
timeline segment, so the chain looks to be computed at compile time. With input, generations beyond the current one are
**provisional**: `eval` computes the neutral chain; the runtime re-derives generation n from the log at the read point of
bar `b_n - 1`, stages it (the existing bar-ahead load) and replaces the provisional segment. The pure functions
`generation(program, log, n)` and `frame_at(c, log, rel)` are what runtime, renderer and debugger all call (the debugger's
"one definition, two callers"). R1, §9.

---

## 3. The performed tier: data model

**Three tiers, three identities.** Authored: the program text, the **program CID** (BTW.md "Canonical form"). Derived:
generations 0..n, each with a CID in the `lineage`, a function of the program alone (or of program and log). Performed:
what was said while it played, the **performance CID** of `(program CID, vocab, log)`.

### 3.1 The input log entry

```json
{"t_bar": 11, "source": "crowd", "key": "hyped", "intensity": 3, "value": 100, "origin": "ui"}
```

| field | rule |
|---|---|
| `t_bar` | integer >= 0, the stamp of §2.3. No wall-clock time anywhere |
| `source` | `crowd` or `mood` |
| `key` | a word of `source`'s vocabulary, or `neutral`. Free text is **impossible**, so a log cannot carry an injection |
| `intensity` | 1, 2 or 3 (default 2) |
| `value` | integer 0..100, the energy in hundredths: a redundant copy for a reader without the table. **Must equal the table's** or the log is rejected |
| `origin` | `ui` or `mcp` in v1. `sensor` and `room` are **reserved** (§8) |

`valence` and `calm` are derived from the table, not stored. A later sensor may add `key: "level"` with a bare `value` and
no word: a `btw.vibe/2` question (13).

**Canonical log.** Sorted by `t_bar` (stable), then **collapsed**: of several entries with the same `source` and `t_bar`
only the last survives (a mis-tap corrected inside a bar leaves no trace, a privacy gain too). An entry that repeats the
standing value is **kept**, since it restarts the 64-bar expiry. Caps: 2048 entries, 64 KiB.

### 3.2 The performance object

```json
{ "format": 1, "type": "djrust.live.performance", "program": "z4EBG9j...", "vocab": "btw.vibe/1",
  "log": [ {"t_bar":3,"source":"crowd","key":"dancing","intensity":2,"value":70,"origin":"ui"}, "..." ] }
```

That is the sealed core. **Canonical JSON** is the share codec's (sorted keys, no whitespace, integers as integers); the
**performance CID** is the share CID (sha-256, CIDv1, base58) of it. As with a saved program (BTW.md "Saved programs") the
**object** adds the rights half and has its own CID, so a new licence is a new object of the same performance:

```text
#! btw-performance 1
#! name: night bus, saturday
#! license: CC0-1.0
#! visibility: private
#! author: did:key:z6Mk...
#! forked-from: z6Pf...          # a parent performance
#! witness: 3 | z4EBGb... | z4EBGc... | z4EBGd...    # generation CIDs the recording saw; outside the performance CID
{ ...the sealed core... }
```

The witness is outside the CID on purpose: it is derived data, and a replay that disagrees is a *finding* ("replay differs
at generation 2"), not an invalid performance.

### 3.3 Links: performance versus program

```text
program link      https://btw.bitwrap.io/#<program CID>.<base64url(deflate(canonical text))>[.<sig>][~<notes>]
performance link  https://btw.bitwrap.io/#perf:<perf CID>.<base64url(deflate({"text": canonical text, "log": tuples}))>
```

A program link is unchanged and **always means the neutral performance** (BTW.md's "opening it reproduces every generation,
CID for CID" stays true for a program that reads no input, and narrows to "the neutral performance" for one that does).
A performance link carries the text and the log as tuples (`[3,"c","dancing",2,"u"]`); `open` checks the program CID
against the text and the performance CID against the log and **refuses a mismatch**, as for programs. Over 2000
characters it says so; a long night is stored (`PUT /api/performances/<cid>`, §7) and linked by `#perf:<cid>` alone. A
private performance is never linked.

### 3.4 Rights and visibility

**Licence** defaults to CC0-1.0 as for programs, and **effective rights are the most restrictive of the performance's and its
program's** (the BTW.md asset rule): a performance of an all-rights-reserved program cannot be CC0, and of a private or `use
restricted` one it is private and `share` refuses, naming the program. **Visibility defaults to private** (programs default
public), because a mood log is personal: saving shows the exact log, and making it public is a second explicit step.
`forkedFrom` is a performance CID (a replay with an edited log, or a continuation), rights inherited unless made stricter; an
accepted program edit is a *program* fork (§3.5). The program's header never carries the vibe (§6.7).

### 3.5 A night with edits

A performance covers **one program text**, replayed from bar 0 (the debugger's fresh-play semantics). A night that accepts
an edit (the guide's flow ends in a fork) is recorded as **a chain**: accepting closes the performance and opens a new one
on the forked program, `forkedFrom` pointing back, `t_bar` rebased to the bar the fork landed on. A hot-reloaded segment can
differ from a fresh replay in `now` lines and ramp phase (the debugger's `Fidelity` corner); the object records
`fidelity: replayed` rather than pretend. A "set" object stringing performances together is out of v1.

---

## 4. Worked example

A 24-line program (proposal syntax; every line outside `evolve` is today's language).

```text
 1  # night-bus: a groove that listens (proposal syntax: crowd, mood)
 2  tempo 124
 3  key am
 4
 5  section groove 32 bars {
 6    deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ | snare ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~
 7    deck A: hihat x*8 vel=60..90 | bass markov(am, 0.6, 5)
 8    deck B: harmony chord(am7) | arp arp(am7, up, 8)
 9    master: reverb 0.15
10
11    evolve every 8 bars {
12      hihat: thin <crowd.calm> seed=3                        # a calmer room, fewer hats
13      arp: rotate 2 | transpose 7 when crowd.mood == hyped   # a hyped room, a brighter arp
14      arp: octave 1 when crowd.mood == wants_drop            # they want the drop: arp up
15      bass: invert when mood == restless and gen >= 2        # one listener, restless
16      stop when gen == 3
17    }
18  }
19
20  section drop 16 bars {
21    deck A: kick x ~ x ~ | hihat x*8 vel=70..100 | bass e2 ~ e2 ~ inst=sub-bass
22    deck B: harmony chord(am7)
23  }
24  arrange groove drop
```

**The night.** Four picks as a person made them, and what the log stores (`t_bar` is the §2.3 stamp):

| what happened | stored entry |
|---|---|
| bar 3, beat 1: "dancing", normal | `{t_bar:3, crowd, dancing, 2, 70, ui}` |
| bar 10, beat 4: "hyped", a lot | `{t_bar:11, crowd, hyped, 3, 100, ui}` (after beat 3, so the next bar) |
| bar 13, beat 1: "restless", normal, solo | `{t_bar:13, mood, restless, 2, 70, ui}` |
| bar 22, beat 2: "wants a drop", normal | `{t_bar:22, crowd, wants_drop, 2, 60, ui}` |

**The reads and the lineage.** Generations of `evolve every 8 bars` start on bars 8, 16, 24 and read at the read points of
bars 7, 15, 23 (`age` is `b - 1 - t_bar`; neutral if absent or 64 or more). The lineage lines of BTW.md gain `reads`:

```text
perf z6Pf..8a  program z4EB..n7  vocab btw.vibe/1  4 entries  fidelity exact
gen scope  start len  reads (at the read point of bar)                                rules that ran
 0  groove     0   8  -                                                               (as written)
 1  groove     8   8  @7  crowd dancing/2 e70 c30 age 4 | mood neutral                hihat: thin 0.3
 2  groove    16   8  @15 crowd hyped/3 e100 c0 age 4 | mood restless/2 e70 v30 age 2  hihat: thin 0
                                                                                      arp: rotate 2 | transpose 7
                                                                                      bass: invert
 3  groove    24   8  @23 crowd wants_drop/2 e60 c40 age 1 | mood restless/2 age 10    hihat: thin 0.4
                                                                                      arp: octave 1
                                                                                      bass: invert
```

(`e` energy, `c` calm, `v` valence. Each generation also has its `cid` and `forkedFrom`, as BTW.md has them. The CIDs here
are abbreviated and nothing was run.) By BTW.md's definition `invert` turns pitches over the part's first note, so applying
it in generations 2 and 3 gives the bass of generation 1 back (inside the MIDI range): its part is the same in generations 1 and 3 though deck A's
net is not (the hats changed). Content addressing at work.

**Replay.** From a shell, with no app (the same pure functions the renderer uses):

```text
$ djctl btw replay night-bus.btw night-bus.perf
program   z4EB..n7  canonical, matches the object
log       4 entries, vocab btw.vibe/1, every value equals the table
gen 1 @7  reads match, cid z4EBGb.. = witness        gen 2 @15  ... = witness        gen 3 @23  ... = witness
replay equal: 3 generations, 0 differences           (perf z6Pf..8a)

```

With no log the same command replays the neutral performance (`thin 0.5` in each generation), and
`djctl btw render night-bus.btw night-bus.wav --perf night-bus.perf` renders the night offline.

Change one entry (bar 11's `hyped/3` to `hyped/2`) and the replay reports `gen 2 @15: crowd hyped/2, the witness saw
hyped/3; cid differs from the witness`. On the site the same night is
`GET /api/render/<program cid>?perf=<perf cid>&format=opus`.

---

## 5. The vibe modal

**Opening it.** A **Vibe** button in the page's slim transport bar and in the desktop btw window's header, present while a
program plays. Shortcuts (to confirm against the page's key map and `app.rs`, as the debugger design says of its own):
`Alt+V` anywhere (matched on `code === "KeyV"`) and a plain `V` outside the editor. The prompt (§5.4) offers the modal; it
never opens it by itself.

### 5.1 Layout

A native `<dialog>` opened with `showModal()` on the web (an egui modal on the desktop): two modes as a radio group, six
word buttons, an optional intensity, a clear button, a footer.

```text
+-- How is it going? ----------------------------------------------- [x] --+
|   ( ) The room          (o) Just me                  <- radiogroup      |
|                                                                          |
|   Just me. I feel...                                                     |
|   +------------+  +------------+  +------------+    keys 1-6 pick:       |
|   |    :D      |  |   (o_o)    |  |   (u_u)    |    1 Energised          |
|   | Energised  |  |  Focused   |  |   Mellow   |    2 Focused 3 Mellow   |
|   +------------+  +------------+  +------------+    4 Sad 5 Restless     |
|   +------------+  +------------+  +------------+    6 Bored              |
|   |   :'(      |  |    :S      |  |    :-/     |                         |
|   |    Sad     |  | Restless   |  |   Bored    |                         |
|   +------------+  +------------+  +------------+                         |
|   How much?   ( ) a little   (o) normal   ( ) a lot                     |
|   [ Clear: back to neutral ]       First read on bar 24.                 |
|   [x] Ask me at section changes     Picks stay on this device.           |
+--------------------------------------------------------------------------+
```

"The room" shows the same grid with the crowd words. Each button is **a glyph and a text label**; the label is the
accessible name and the glyph is decoration (`aria-hidden`).

| crowd key | glyph (code point, CLDR name) | fallback | solo key | glyph (code point, CLDR name) | fallback |
|---|---|---|---|---|---|
| `hyped` | U+1F525 fire | `\o/` | `energised` | U+26A1 high voltage | `:D` |
| `dancing` | U+1F57A man dancing | `o/~` | `focused` | U+1F9D0 face with monocle | `(o_o)` |
| `chilling` | U+1F60E face with sunglasses | `B-)` | `mellow` | U+1F60C relieved face | `(u_u)` |
| `drifting` | U+1F636 face without mouth | `(~_~)` | `sad` | U+1F622 crying face | `:'(` |
| `leaving` | U+1F6AA door | `[>` | `restless` | U+1F62C grimacing face | `:S` |
| `wants_drop` | U+1F4A5 collision | `\!/` | `bored` | U+1F611 expressionless face | `:-/` |

The label is the key with a capital (`wants_drop` is "Wants a drop"). The fallback is drawn when the platform has no emoji
font for the glyph. Labels are localisable; the `key` is the stable English word programs use.

### 5.2 One tap

Choosing a word **logs at once and closes**; nothing needs confirming and the music is never interrupted. The intensity control
is the optional part: set it first and it applies to the tap; it defaults to normal and is remembered for the session, as is the
mode. Focus returns to the Vibe button and a polite live region outside the dialog says "Logged: room, hyped, a lot. First read
on bar 24." The button carries a small chip with the standing pick ("room: hyped") for as long as it is in force.

### 5.3 Keyboard, screen reader, focus, motion

| concern | behaviour |
|---|---|
| Open and focus | the button, `Alt+V`, `V`; focus lands on the standing word's button, else the first |
| Words | `button`s in a grid with a roving `tabindex`; arrows, `Home`, `End` move; `Enter` / `Space` pick; `1` to `6` pick directly |
| Mode, intensity | radio groups (`role="radiogroup"`, arrows); `Tab` moves group to group |
| Close | `Esc` or a backdrop click closes **without logging**; focus returns to the opener |
| Trap | modal `<dialog>` semantics make the page `inert` behind it; the music keeps playing |
| Names | every button has a visible text label as its accessible name (never title-only or emoji-only); `aria-labelledby` on the dialog |
| Targets, contrast | at least 44 x 44 px; AA in light and dark; the selection is not by colour alone (a check mark and text) |
| Motion | no pulsing, shaking or confetti; `prefers-reduced-motion: reduce` removes the open, close and chip transitions; "first read on bar N" is text, never animation |
| No time limit | the modal never closes itself; the chip stays until dismissed or the next one is due, never under 10 seconds |

### 5.4 The prompt at section changes

An **unobtrusive chip** beside the transport, not a modal, never taking focus (`role="status"`, polite):
`How's the room?  [ Tell me ]  [ Not now ]  [ Stop asking ]`.

| rule | value |
|---|---|
| when | a new `section` starts; with no sections, every 32-bar phrase; and, if the last pick is over 24 bars old, two bars before a generation's read point |
| only if | the program **reads input**, **or** the guide pane is open. A program that cannot use the answer is never asked |
| never | in the first 4 bars; within 16 bars of the last chip; while the dialog is open; within 3 seconds of a keystroke in the editor |
| "Not now" | snoozes 32 bars |
| "Stop asking" | off for good: `localStorage` `btw.vibe.prompt = off` (every access in try/catch, falling back to session memory); the same switch is the modal's footer checkbox and a line in settings |

---

## 6. The guide pane

A collapsible **side pane** (a Guide button, `Alt+G`) that turns recent picks into *prompt guidance*. It does not write
music and does not call a model to decide what to suggest: it reads the log, applies a rule table (§6.3) and shows two or
three **suggested instructions** to send as they are, edit first, or ignore. A sent instruction goes to compose or edit
with the context bundle (§6.5) and returns as a diff to preview and accept as a fork (§6.6): the debugger's flow.

### 6.1 Wireframe

```text
+-- Guide ------------------------------------------------- [window: 32 bars v] -+
| Listening to   room: drifting (16 bars)      me: mellow (4 bars)                |
| energy (room)  _.-""-._.--.___   falling: 70 -> 20 over 32 bars                 |
| Suggested                                                                       |
| +-----------------------------------------------------------------------------+ |
| | 1  room drifting for 16 bars                                          [x]   | |
| |    "Build a breakdown for the last 8 bars of this section into a bigger     | |
| |     drop."          [ Send ] [ Edit ]   ( ) now  (o) and keep listening     | |
| +-----------------------------------------------------------------------------+ |
| | 2  me mellow (4 bars): "Strip the hats and open the pad." [ Send ] [ Edit ] | |
| +-----------------------------------------------------------------------------+ |
| Or write your own: [ ____________________________________ ] [ Send ]           |
| Proposal (1 statement)                      base z4EB..n7 -> fork               |
|  + 31  at bar 24: riser(8)                                                      |
|  footprint: bars 24-31 . master . ok (inputs held at their standing values)     |
|  [ Preview ] [ A | B | alternate ] [ Accept as fork ] [ Reject ]                |
| Sent to the model: program, bar 21, section groove 21/32, last 32 bars of picks |
|   (what, exactly) v                                                             |
+---------------------------------------------------------------------------------+
```

With no picks: "No picks yet. Tap Vibe (Alt+V) to say how the room or you feel." and no suggestions. Nothing is invented.

### 6.2 What the pane knows

**The window** is the last **32 bars** (16 or 64 selectable) plus each source's *standing* entry even if older (up to its 64-bar
expiry). From the log and the position the guide computes `features`, a pure function with no model: the current word, energy
and `run_bars` (how long the word has been in force; repeats extend it); the energy `trend` over the window (rising at +20 or
more, falling at -20 or less, else flat); `n_picks` and `last_pick_age`; `disagree` (crowd and mood energy 40 or more apart
within 8 bars); and, from the program and `marks`, the section, `bars_to_section_end`, `bar_in_phrase`, `gen` and the active roles.

### 6.3 The rule table

Each rule has an id, an evidence condition, one or two instruction templates and `requires`, so it never suggests what
the program cannot do ("the hats" only if a `hihat` part plays). Instructions are fixed English with slots filled from the
program (section and role names).

| id | evidence | suggested instruction | requires |
|---|---|---|---|
| G01 | `crowd drifting`, run >= 16 | "Build a breakdown for the last 8 bars of this section into a bigger drop." | a later section, or end >= 8 bars away |
| G02 | `crowd wants_drop`; next section not a drop, or end <= 8 | "Add a riser into the next section and make its first bar hit harder." | |
| G03 | `crowd wants_drop`, already in a drop | "Strip to kick and bass for 2 bars, then bring everything back." | kick or bass |
| G04 | `crowd hyped`, run >= 8 | "Hold this level: add a hi-hat roll every 4 bars and keep the arp bright." | hihat or arp |
| G05 | `crowd dancing`, run >= 8, trend flat | "Keep the groove; vary only the top line, rotating the melody every 8 bars." | melody or arp |
| G06 | `crowd chilling` | "Soften it: kick on the half bar, open the pad's filter." | kick |
| G07 | `crowd leaving`, intensity >= 2 | "Change the groove: go half-time and swap the bass pattern." | bass |
| G08 | crowd trend falling, not `leaving` | "Raise the energy: a four-bar build with a filter sweep, then a fill." | |
| G09 | crowd trend rising, end <= 4 bars away | "Make the next section a drop with a heavier bass." | a next section |
| G10 | `mood mellow` | "Strip the hats and open the pad." | hihat or pad |
| G11 | `mood restless`, run >= 4 | "Add movement: rotate the hats every 4 bars and sweep the filter over 8." | hihat |
| G12 | `mood sad` | a) "Stay with it: slow the tempo by 4 BPM and add reverb." b) "Lift it gently: brighten the arp an octave." (a first) | |
| G13 | `mood bored`, run >= 8 | "Introduce something new: a counter-melody on deck B and a new fill every 4 bars." | deck B |
| G14 | `mood focused` | "Reduce distraction: remove the lead and hold a steady pulse; no section change for 32 bars." | |
| G15 | `mood energised`, run >= 8 | "Match it: hats at sixteenths and an octave-up arp." | hihat or arp |
| G16 | `disagree` | not an edit: asks whose vibe to follow (room or me), then the rules above run for that source | |
| G17 | all picks older than 64 bars | not an edit: "No recent picks", with a Tap Vibe button | |

Each card also has **now / and keep listening**. "Now" sends the instruction as written. "And keep listening" appends "and
make it respond to the room: use `thin <crowd.calm>` or a `when crowd.mood == ...` rule in an evolve" (or the solo form), so
the fork **turns a one-off edit into a standing reaction** the program carries. That is how the guide feeds the language:
the room steers the author, and the author makes the program listen.

### 6.4 Ranking

Deterministic, no model: the same log and position give the same cards.

```text
score = 0.40 * min(run_bars / 16, 1)   + 0.20 * (intensity / 3)   + 0.20 * recency (1 at the last pick, 0 at the window start)
      + 0.20 * fit (the section and phrase position suit the rule)   - 0.50 * recently_sent (id sent or dismissed in 32 bars)
ties by rule id
```

Show the top three whose `requires` hold and whose score is at least 0.25; a dismissed card (`x`) stays away 32 bars. The
weights are starting values to tune from use, not findings (question 9).

### 6.5 The context bundle

`btw.crowd.context/1` is the debugger's `btw.debug.context/1` with the same keys and ordering rule (static prefix first for
caching, then the program, then position and instruction; sorted keys, no timestamps) plus `performance` and `marks`.

```json
{
 "format": "btw.crowd.context/1", "extends": "btw.debug.context/1",
 "program":   { "cid": "z4EB..n7", "text": "<as written; canonical if foreign>", "line_hashes": ["c1f2", "..."] },
 "selection": { "kind": "section", "line": 5, "statements": [5] },
 "position":  { "bar": 21, "step": 0, "section": "groove", "bar_in_section": 21, "bars_in_section": 32,
                "phrase": 3, "bar_in_phrase": 6, "generation": [["groove", 2]] },
 "paused":    null,
 "marks":     { "active_lines": [6, 7, 8], "active_roles": {"A": ["kick","snare","hihat","bass"], "B": ["harmony","arp"]} },
 "performance": {
   "vocab": "btw.vibe/1",
   "standing": { "crowd": {"key":"drifting","intensity":2,"energy":20,"since_bar":5,"age_bars":16},
                 "mood":  {"key":"mellow","intensity":2,"energy":30,"valence":70,"since_bar":17,"age_bars":4} },
   "log_window": { "from_bar": 0, "to_bar": 21, "entries": ["... at most 64 ..."] },
   "reads": [ {"gen":1,"at_bar":7,"crowd":"dancing/2","mood":"neutral"} ],
   "next_read": { "bar": 23, "generation": 3 },
   "features": { "trend": "falling", "run_bars": 16, "disagree": false }, "guide_rule": "G01" },
 "instruction": "Build a breakdown for the last 8 bars of this section into a bigger drop.",
 "policy": { "scope": "section", "allow_widen": false }, "limits": { "max_statements_changed": 12, "max_bytes": 2048 },
 "instruments": ["..."], "stdlib": { "import": "std/transitions", "index": ["..."] }, "grammar": "<cheat-sheet incl. Listening>"
}
```

**Aligned with the debugger:** every key above except `performance`, `marks` and `guide_rule` is the debugger's (plus its `frame` and
`why` when the debugger exists); a foreign program goes in canonical and fenced as data, as there. **The log is injection-free by
construction** (closed vocabulary, integers); the instruction is the user's own text. **What leaves is shown:** "Sent to the
model" expands to the `performance` block, and nothing is sent before Send.

### 6.6 Preview and accept, as a fork

```text
 tap a card / edit it --> context bundle --> [ model, outside the app ] --> Proposal(ops) --> apply -> check -> footprint
        ^                                                                                              |
        |                                                                                              v
   accept / reject <-- audition A/B <-- preview (hold-loop) <---------------------------- guardrails (BTW-DEBUGGER §6.8)
        |
        v
   commit as fork: forkedFrom = base CID, provenance header, closes this performance and opens the next (§3.5)
```

| debugger concept | here |
|---|---|
| context bundle, `propose` (ops, apply, check, footprint), `preview` (hold-loop), `audition`, `accept`, `reject`, `undo` | unchanged. The **footprint is computed with inputs held at their standing values** (and under neutral) and labelled; the live log keeps being appended during a preview; **accept** also cuts the performance (§3.5) |
| `frame_at(c, rel)` | `frame_at(c, log, rel)`; an empty log is today's function |
| the `debug` noun, `program_callable = false` | the `crowd` and `mood` nouns, same flag: a program cannot set its own input |
| compose **edit mode** (§6.9 there) | takes the same `context`; same limits and repair loop |

**If the debugger has not landed**, the guide still works on what exists: whole-program compose
(`POST /api/compose` with `base_cid`, already in the route), the client canonicalises the result and shows a **line diff of
canonical text** instead of an ops diff and footprint, and accept is an eval plus a fork. The route needs a public, CC0,
unrestricted base, so a **private or restricted program cannot use the hosted guide**; the MCP-driven path (the user's own
Claude) is unaffected. That is the v1 path (§9); it moves to ops when the debugger's P5 exists.

### 6.7 Provenance

```text
#! forked-from: z4EBG9j...
#! edit: Build a breakdown for the last 8 bars of this section into a bigger drop.
#! edit-by: gemini-3.8-flash (compose)
```

The header carries **the instruction, never the evidence**: no picks and no rule id (an id would reveal "mellow" or
"restless"). The program is a public CC0 object; the vibe belongs to the performance.

---

## 7. HTTP and MCP surface

The site already has the pieces (DESIGN.md: sessions hub, compose, MCP). The picks live **in the page** (the `BtwRun` log,
`sessionStorage` for a reload); the server holds only what the page reports, as it does for sessions today (memory, 10
minute TTL). Nothing is stored server-side until the user saves a performance.

### 7.1 Routes

| route | auth | notes |
|---|---|---|
| `POST /api/session/<id>/report` (existing) | as today | the body gains `inputs`: `{vocab, bar, standing, log_window (<= 64), reads (<= 8), next_read}`, **sent only when the page's "MCP may read and set the vibe" toggle is on** (default off) |
| `GET /api/session/<id>/events` (existing) | as today | a new event `input` `{request_id, source, key, intensity}`; the page applies it, appends `origin: mcp`, and answers `POST .../reply {request_id, entry}` or a refusal |
| `PUT /api/performances/<pcid>` | cookie or bearer (signed in) | body is the performance object; accepted iff its canonical CID is `<pcid>`, the program is in the store (public, CC0, unrestricted) or its text is inlined and checks, `vocab` is known, every entry validates (§3.1), caps hold. 20 an hour per user, 40 a day per IP. `visibility: public` needs a public program |
| `GET /api/performances/<pcid>` (and `.json`) | public if public; private to its owner | `{pcid, program, vocab, log, meta}` |
| `GET /api/render/<cid>?perf=<pcid>&format=opus\|wav` | as render today | rendered with the log; cache key `(cid, pcid, format, sr)`; runs `btwd render --perf` |
| `POST /api/compose` (existing) | cookie or bearer | gains `context` (an object, <= 8 KiB: `performance`, `marks`, `position`). In whole-program mode it is rendered into the prompt; in edit mode it is the `frame` |
| `POST /mcp` (existing) | bearer | the tools below |

### 7.2 MCP tools

Descriptions are written for an LLM, as the existing ones are: *"crowd_set reports what the room is like so the next
generation that reads the crowd uses it. Call it only when the user told you what the room is like or asked you to test.
Words: hyped, dancing, chilling, drifting, leaving, wants_drop, neutral."*

| tool | args | returns | auth |
|---|---|---|---|
| `crowd_get` | `session?`, `window_bars?` (default 32, max 256) | `{vocab, bar, now: {key, intensity, energy, calm, since_bar, age_bars, origin}, log, reads, next_read: {bar, generation}, program_cid}`; `now.key` is `neutral` when none | bearer, as `get` |
| `crowd_set` | `key`, `intensity?` (1..3, default 2), `session?` | `{ok, entry, first_read: {bar, generation}}` | bearer, as `eval` |
| `mood_get`, `mood_set` | as above | the same for the solo source (`mood_get` adds `valence`) | as above |

- **Consent.** Reads and sets return `not_permitted` ("the user has not enabled 'MCP may read and set the vibe' on the page;
  ask them to turn it on in the MCP panel") unless the page's per-session toggle (default off) is on. A person's mood is not
  Claude's to read or write until they say so.
- **Origin** is always `mcp`; a caller cannot claim `ui`. The page toasts "Claude set the room to hyped" with Undo (which
  logs the previous standing value, or overwrites within the same bar by the collapse rule).
- **Errors** are the existing tool errors `{error: {code, message, retry_after, details}}`: `auth_required`, `no_session`,
  `not_permitted`, `bad_request` (an unknown key lists the vocabulary), `rate_limited`. `tools-list.json` is updated and reviewed.
- **Desktop:** the same actions are registry nouns `crowd` and `mood` (`get`, `set`, `log`, `clear`): `djapp_crowd`, `djapp_mood`,
  `djctl app crowd set hyped intensity=3`. Both are `program_callable = false`, so `do crowd set` in a program fails `check`.
  `djctl btw replay` and `btwd render --perf` need no app and no session.

### 7.3 Limits and size caps

| what | limit |
|---|---|
| `*_set` | 20 a minute and 600 a day per user; at most one entry per source per bar survives (collapse), so a flood costs nothing downstream |
| `*_get` | the existing check limiter (60 a minute); response <= 32 KiB; `window_bars` <= 256 |
| one entry | <= 256 bytes of JSON; `key` <= 16 characters and in the vocabulary |
| a performance | <= 2048 entries, <= 64 KiB canonical JSON; a link over 2000 characters is stored instead |
| compose `context` | <= 8 KiB, <= 64 log entries; same limiter and spend cap as any compose, each repair round counted |
| telemetry | picks are **not** an event kind; `telemetry.Kinds` is not extended |

---

## 8. Trust, abuse, rooms and privacy

### 8.1 Who may report

A crowd report from a public room is **low trust**: anyone can type "leaving", and a hostile voter can sink a set. The model
is a trust ladder, and v1 stops at the first two rungs.

| rung | who | trust | status |
|---|---|---|---|
| 1 | **The person running the program** (a solo listener, or a DJ reporting the room) | full: their own page and log | **v1** |
| 2 | **Claude, for that person** (`origin: mcp`, consent toggle) | as the person, recorded as `mcp` | **v1** |
| 3 | **Guests in a room the host opened** | low: anonymous, aggregated, host-vetoed | v1.1 |
| 4 | A sensor on the host's device (`origin: sensor`) | a derived scalar only, on-device | later |

### 8.2 Rooms (a v1.1 sketch, so v1 leaves room for it)

The host opens a room (`POST /api/room`, signed in) and gets a short code (`K7QM-4T`). Guests open `/r/K7QM4T`: only the six
crowd buttons, no program, no audio, no sign-in, and a random per-room token in `sessionStorage`. The server keeps votes 30
seconds and sends the host **one aggregate** every 5 seconds (`{n, plurality, spread}`); the host's page makes at most one log
entry per bar from it, `origin: room`.

- **Only the aggregate enters a performance,** so a replay does not depend on how the room voted and no guest is identifiable
  from a shared log. Guests send crowd words only, never mood.
- **Quorum and trim:** at least 3 distinct tokens in the window; the plurality ignores the single most distant vote.
- **Sybil and flood:** one vote per token per 10 seconds; at most 5 tokens per /24 (/64 for IPv6); a room vote budget.
- **Host controls:** mute, DJ-only mode, close, and a **veto** (a host pick in the same bar overrides the aggregate by collapse).
  The room is advisory: a room full of bots still votes, and the host decides.

### 8.3 Injection and cost

The log is closed-vocabulary integers, so a poisoned log cannot carry text to a model; the only free text a model sees is the
user's own instruction. A private or restricted program never reaches a hosted model (the debugger's R7). Every compose with
`context` goes through the existing limiter and spend cap.

### 8.4 Privacy

1. **Anonymous.** An entry has no user id, device id, IP or **wall-clock time**, only bars. A performance's author is the
   user's choice, and `created` is omitted unless they add a date.
2. **Local by default.** Picks live in the page's memory and `sessionStorage` (so a reload keeps the night; cleared with the
   tab). "Clear picks" empties both. Nothing is sent when a pick is made.
3. **Only logged values go into a performance, and only if the user saves or shares it.** Saving shows the exact log;
   visibility is private until a second step; the program header never carries the vibe (§6.7).
4. **Leaving the device** happens on three explicit acts only: Send to the guide (which shows the block), turning on the MCP
   toggle, and saving or sharing a performance.
5. **No camera or microphone in v1.** If a sensor comes: processing on the device; only a derived scalar (a loudness band, a
   motion energy) becomes an entry, `origin: sensor`; never raw frames or audio, never stored; a per-session revocable
   permission and a visible indicator. A camera that sees faces is out of scope for anything called a "crowd report".

---

## 9. v1 scope, build order, risks

### 9.1 The cut

**v1 = the core, the web page, MCP and the offline renderer, for a single user and a DJ.** The desktop gets the runtime and
the registry nouns (so Claude can use it there) but not the modal and pane (v1.1). Not in v1: rooms and guests, sensors,
inputs in control lines, a model-ranked guide, the ops/footprint diff if the debugger has not landed, chained "set" objects,
share cards for performances.

### 9.2 Build order and effort

Engineer-days for someone who knows the repo, plus or minus half. Steps 1 to 4 are the spine and are useful alone.

| # | Step | Where | Depends | Days |
|---|---|---|---|---|
| 0 | **Spikes** S1 to S3 below; the corpus grep for `crowd` / `mood` | `djlive` | | 1 |
| 1 | `djlive::input`: vocab table and golden, `InputLog`, `stamp`, `snapshot_at`, canonical log, performance CID, validation | `djlive` | 0 | 3 |
| 2 | Language: lexer, parser, printer, `check` (types, bound, alphabet, per-symbol orbits), evolve over `&dyn InputView`, `generation(program, log, n)`, `reads` in the lineage; corpus CIDs pinned | `djlive` | 1 | 5 |
| 3 | Runtime: `Live::push_input`, the read point, the provisional tail, `frame_at(c, log, rel)`; `btwapi` and `djweb` exports | `djlive`, `btwapi`, `djweb` | 2 | 4 |
| 4 | Offline: `btwd render --perf`, `djctl btw replay`, `btw_render` takes a log; the record-replay test | `btwd`, `djhost` | 3 | 2.5 |
| 5 | Web: the modal, the chip, the standing-pick chip, a11y, `sessionStorage`, the session report's `inputs` | `btwsite/web` | 3 | 4 |
| 6 | Performance object: store, `PUT` / `GET`, `#perf:` link, save and share UI, rights rules | Go, page | 1, 5 | 3.5 |
| 7 | MCP: four tools, the consent toggle, the `input` session event, `tools-list.json`; desktop registry nouns | Go, `djhost` | 3, 5 | 3 |
| 8 | **Guide pane v1**: features, the 17 rules, ranking, the pane, the bundle, whole-program compose with `context`, canonical line diff, accept as a fork | page, Go | 5, 7 | 6 |
| 9 | Guide on the debugger's ops, footprint, preview and A/B | `djlive`, page | 8, debugger P5 | 3 |
| 10 | Desktop btw window: the modal and the pane | `djapp` | 3 | 5 |

**v1 is steps 0 to 8: about 32 days.** Step 9 and the desktop UI are v1.1. The cheapest first demonstration is steps 1 to 4
(about 15 days with the spike): a program that reads a hand-written log, replays exactly and renders, with no UI at all. It
settles the hard questions.

**Spikes (hours each, before step 2).** **S1:** confirm the runtime's clock (`Live.line`, the 0.75 apply window) and how a
`loop` song counts bars; `t_bar` needs a monotonic played-bar count, else a lap counter (question 11). **S2:** confirm how
the evolve chain is computed today and that a generation's nets can be compiled and staged in the apply window of the bar
before it. **S3:** plumb a view argument through `Compiled::generation_cid(scope, n)` and `evolve::nth` without changing any
existing CID (the corpus test is the check).

### 9.3 Risks

| | Risk | Mitigation |
|---|---|---|
| **R1** | The chain becomes lazy (generations beyond the current one are provisional): touches `compile.rs`, `runtime.rs`, the markers and the debugger's `frame_at` | S2 first; the provisional tail is the neutral chain, so `eval` still has a full chain to show; one pure `generation(program, log, n)` for runtime, renderer and debugger |
| **R2** | A stamp disagrees with the clock under hold, seek or loop | one Rust stamp function; held picks collapse; S1 |
| **R3** | A mood log leaks (a public performance, a header, a link) | private by default; the exact-log preview; no vibe in headers; no wall-clock time; the consent toggle |
| **R4** | Suggestions feel wrong or patronising (`sad` especially) | the rules are a visible table; two stances for `sad`; every card is editable and ignorable; nothing auto-applies |
| **R5** | Input weakens "same text, same net" | T2; the neutral performance is the default; a test pins the corpus CIDs |
| **R6** | Another agent sets a person's mood; vocabulary drift breaks old logs | consent, `origin: mcp`, toast with Undo, rate limit; the log names `vocab`, `btw.vibe/1` is frozen, `value` must match the table |
| **R7** | The prompt chip annoys; `crowd` / `mood` break a saved program | only when the program can use the answer, capped, one switch off; the corpus grep and a named fix |

---

## 10. Test plan

| Test | Asserts |
|---|---|
| **Neutral equivalence** | with an empty log a reading program's lineage equals that of the same program with each reference replaced by its neutral constant; for every program that reads no input, **every existing corpus CID is unchanged** |
| **Record-replay** (`tests/perform.rs`) | on the fake clock run `Live` with random picks (times within bars, both sources, `ui` and `mcp`), save, replay from the object alone: identical generation CIDs and command log |
| **Stamp and read boundaries** | beat 2.999 stamps k and 3.0 stamps k + 1; a pick before the read point of bar `b - 1` is seen by the generation starting on `b`, one after it is not; age 63 reads the pick and 64 reads neutral |
| **Canonical log** | permuting entries within a bar, adding an overwritten same-bar pick, or reformatting the JSON keeps the CID; changing one intensity changes it; a `value` that disagrees with the table is rejected |
| **Totality and `check`** | a fuzz over the grammar never loops or panics; an unbounded reading evolve is an error; the alphabet is 19 x 7 = 133 for the worked example; the wrong-vocabulary word, `transpose <crowd.energy>` and `crowd.valence` fail with their messages |
| **Wasm parity** | Node loads the built `djweb` wasm and compares program CID, lineage with `reads` and performance CID against `btwd` over a corpus of programs and logs |
| **Renderer** | `btwd render --perf` equals the offline render of the replayed generations; the cache key includes the performance CID; no log renders neutral |
| **Modal** (Playwright, headless Chrome) | open by button and `Alt+V`; labels are the accessible names (accessibility-tree snapshot); arrows, digits, `Esc` (nothing logged), focus return; live-region text; reduced-motion emulation gives no transition; 44 px targets |
| **Prompt policy** | table-driven on a fake clock: never in the first 4 bars, never for a program that reads no input (unless the guide is open), 16-bar spacing, "Not now" snoozes 32, "Stop asking" persists and survives a blocked `localStorage` |
| **Privacy** | a pick makes **no network request**; the program header after an accepted edit has no vibe and no rule id; a performance has no wall-clock field; an unknown `key` or a free-text field is refused by `PUT` |
| **Guide** | each of G01 to G17 has a positive and a negative fixture log; ranking is stable (ties by id); `requires` hides a rule the program cannot satisfy; the bundle is byte-identical for equal inputs and never carries a private or restricted program |
| **MCP and store** (Go, fake session) | `tools/list` golden; `auth_required`; `not_permitted` with the toggle off; `bad_request` lists the vocabulary; `origin` always `mcp`; rate limit; a set is visible to the next get and the next read; `PUT` refuses a CID mismatch, an over-cap log, a public performance of a private program; a forged link is refused |
| **Hosted compose with context** | the fake model: good, repairable, unrepairable; a context over 8 KiB refused; the limiter counts it |
| **UI shots** | the `layout_audit` and site smoke scripts: both modal modes, the chip, the guide with three cards, a proposal diff |

---

## 11. Open questions, with my recommendation

1. **`calm` as sugar for `1 - energy`?** Yes: `thin` removes, so `thin <crowd.calm>` reads naturally. The alternative, a `1 -`
   prefix, grows the grammar for one use.
2. **Add `restless` to the crowd words?** No. Your example said "crowd: restless"; `drifting` and `wants_drop` cover it and
   `restless` is already a solo word. Disjoint words make the type error useful.
3. **Neutral always the middle (0.5)?** Yes in v1: `thin <crowd.calm>` is `thin 0.5` at neutral, so write programs whose
   neutral behaviour you like. A per-use default (`<crowd.calm ?? 0.1>`) is a later, small extension.
4. **Expiry of 64 bars; read in the bar before, at beat 3.** Recommended: it is the runtime's own apply window, and the cost is
   one bar of latency, shown in the UI. A per-source expiry is a `btw.vibe/2` change.
5. **Performances private by default (CC0 once public); mid-night edits as a chain** of performances, not one object with a
   program track. Both recommended: simpler, exact per segment, and an accepted edit is a fork.
6. **Intensity and the prompt.** One intensity control above the grid, default normal, so the common path is one tap; the
   prompt on only when the program reads input or the guide is open, with one switch off.
7. **Inputs in control lines** (`when mood == restless` on a timed line). Not in v1: it needs a read at the line's fire bar and
   more tests. Do it once `evolve` is proved.
8. **MCP consent.** Required, per session, default off, for reads as well as sets.
9. **Model re-ranking in the guide?** No in v1: rules are free, offline, testable and legible. A model may later *rephrase* a
   card, never re-rank, and never see more than the bundle shows.
10. **Link shape.** A distinct `#perf:` form, so a program link keeps meaning "the neutral performance".
11. **`loop` songs.** `t_bar` counts played bars forever and generation boundaries map to lap-offset bars; S1 confirms, and if
    the runtime wraps its line the stamp gains a lap counter.
12. **Does the guide need the debugger?** No: v1 uses whole-program compose and a canonical line diff, and upgrades to ops,
    footprint, hold-loop preview and A/B when the debugger's P5 lands.
13. **Room trust (v1.1) and sensors (later).** Quorum 3, trimmed plurality, host veto, aggregate-only in the log; a sensor is a
    `btw.vibe/2` with `key: "level"`, `origin: sensor`, on-device scalars only.
14. **Two nouns or one?** Two (`crowd`, `mood`) sharing an implementation, to give the tools the names you asked for. One
    `vibe` noun with `source=` is simpler for the registry, at the price of `vibe_set source=crowd`.

---

## 12. Appendix: what changes in existing docs

- **`docs/BTW.md`:** a "Listening" section under Evolving (§2.1 to §2.3) and the grammar of §2.2; the required bound under "Total,
  and the bound"; `reads` under "CIDs and lineage"; the performance link under "Share links" (a program link is the neutral
  performance); `generation {log}` and `replay` under "The `btw` noun"; inputs in control lines under "Not done".
- **Registry and `docs/CONTROL.md`:** the `crowd` and `mood` nouns, `program_callable = false`. **`BTW-DEBUGGER.md`:** `frame_at`
  takes a log; the bundle gains optional `performance` and `marks`; the footprint is computed with inputs held.
- **`btwsite/DESIGN.md`:** `SessionEvent` `input`, `SessionState` `inputs`, the routes and tools of §7, compose `context`, the
  `#perf:` link, and the Listening section in the generated cheat-sheet so the compose prompt knows the vocabulary.
  **`docs/TESTING.md`:** the record-replay and neutral-equivalence suites.
