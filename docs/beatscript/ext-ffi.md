# The engine side: calling the generator (`gen@v`), pinning, eject and provenance

Roadmaps 6668 (load seeded-pure generated tracks by reference), 6670 (BeatScript's FFI to beats.bitwrap), 6680 (recipe and
expanded CIDs). Built on `spec/gen-reference`. The short reference is BEATSCRIPT.md, "Two halves: structure and engine" and "Call the
generator: `gen@v`"; this note has the decisions and what is left. It also has `seed@2`, seeds v2: the beats generator's track
with its genre's swing ("`seed@2`: seeds with their swing").

## The two halves

A program is **structure** (its own source: settings, parts, `let`, `def`, `section`, `arrange`, templates, a stored
program's sections) and **engine calls** (what it asks the engine for: a generator's program, a move, a verb of the control
registry, a ramp on a strip). Structure is canonical and content-addressed as it is. An engine call is a foreign-function
call: the text records the call, never its result, so a call that makes music must be pinned and versioned, and the language
writes its result out as structure before anything is lowered. The two never share a line: a part line holds notes, a control
line a control call; a generator's result feeds the parts (a pattern, a section) and nothing else.

## `gen@v`: the generator as a function

```text
gen@2 GENRE seed=N [bpm=B] [key=K] [length=short|standard|long] [section=NAME] [part=ROLE]
```

**Which generator.** The seeded generator, `beatscript_api::generate`: the one beatscript.app's compose panel runs as "seeded, no
model", the MCP `generate` tool, `POST /api/generate`. It already shipped versioned (`#! generator: seeded@1`, then
`seeded@2` on 2026-11-22, which wrote chords as Roman numerals). `gen@N` is `seeded@N`, and its params are seeded's own
(`seed`, `bpm`, `key`, `length`), so nothing is invented. `seed(...)` is a different generator (the beats sequencer's `compose`,
byte for byte) and stays what it was; a first cut of `gen` on `compose` (exposing its densities and traits) was set aside for
this.

**The version is in the text.** `gen@1`, `gen@2`; a bare `gen` is `gen@2` (the version this BeatScript added `gen` with), and
the canonical text always writes the version. A `#! seeds:` or `#! templates:` line is a comment and is not in the CID, so a
version there cannot pin anything; a version in the call is in the CID.

**Every version stays callable.** `gen@1` is `seeded@1`, restored from 425458c^ as a frozen module
(`crates/beatscript-api/src/generate_v1.rs`) beside the current printer. A generator change that would print another program for the
same inputs must be a new version, and the old printer stays. `crates/beatscript-api/tests/fixtures/gen-pins.txt` pins what a sample of
calls (both versions) resolves to; the test fails when one moves. One thing the freeze cannot hold: the printer filters its
instrument choices by the web catalogue (`snapshot.json`), which changed twice after seeded@1 shipped, so a `gen@1` call today
may name other voices than beatscript.app printed for the same seed before. Matt's dubstep 54876 (seeded@1) is in the pins as
this build resolves it.

**A call resolves to a pinned CID.** The resolution of a call is the program the generator prints for it, and its CID (of its
canonical text) is the pin: what `generate` reports and stores for the same request. `check` says it at the call (an Info
diagnostic: `gen@2 techno seed=42 section=drop part=bass -> z4EB... (seeded@2): generated in the program's key am and at its
tempo 124`), `refs::sites` gives it to the tools (`resolved`), and eject writes it into the `# from:` line.

**Decided: the canonical text carries the call, not the CID.** The version pins the generator and the params are the call, so
the call alone is total and deterministic, and the program's CID never depends on running the generator. Writing the resolved
CID into the text (a lockfile in the program) would make canonicalisation depend on the engine and the text unreadable; the
pins test is the lock instead, held by a mechanism, not by the text.

**It fits the program it is in (6673).** Unless the call says `bpm=` or `key=`, it is generated at the program's top-level
`tempo` and in its `key` (the seeded generator takes both as inputs, so this is generation, not transposition after the fact).
Two seeds crossed with `morph`/`xfer` are in one key and never clash. The fitting is part of the resolution: the program text
says the key and tempo, so the same text is the same request, the same generated program and the same CID. A call in a program
with no `key`/`tempo` gets the genre's own. A section's own `key` does not refit a call (the top-level one does): taste call.

**Where it stands decides what it is written out as.**

| place | written out as |
|---|---|
| `let bed = gen@2 ... section=S part=R` | the notes R plays in S, a let a bar: `bed_1` ... `bed_k`, `let bed = <bed_1 ... bed_k>` (the shortest run of bars it repeats, a one-step bar as a group `[c3]` so a mutation can take it) |
| `deck B: arp gen@2 ... section=S` | a stored reference to the generated program (`<cid>.S`), written out by stored.rs: the part's pattern and params, the line's own params win |
| `section X from gen@2 ... section=S` | the same: S's parts, its length, its key when it differs; never its control lines (a warning names them) |
| `arrange from gen@2 ...` | the whole program, less the `tempo`, `key` and `swing` lines the program says itself, under its `# from:` line |

The generated program is the source for the stored stage only for as long as the check that resolved the call runs
(`gen::with_generated`): a hand-written `<cid>.drop` of a generated program resolves only where the host has that program, as
before. Generated programs are not carried by a link (the call makes them again).

**A let is notes, a let a bar** because that is what the arrange mutations need: `~ bed_* rot 2` reaches every bar, as for a
theme (forms.md). The let form loses the part's velocities and params (a let is a pattern), as a stored program's let does.

## Arrange mutations reach a let of lets

Making a referenced part mutable found a bug: a section read only the names written in its own lines, so mutating a let's bars
(`~ ode_to_joy_* morph ...`) changed nothing a part played when it played the alternation let (`melody ode_to_joy`), which is
how every form template writes its themes. Every rondo refrain was the same net. mutate.rs now reads through lets (a let that
names lets a play renamed gets a derived let for that play), and `morph`/`xfer`/`mask` read a let that is one `transpose(x, n)`
as `x`'s steps moved. **Taste call:** the six forms and the three reference programs (canon-of-fate, full-fathom-five,
rag-rondo) now sound as their plans say; their CIDs did not move, their music did. The commit is separate (8a8624f) so it can be
dropped.

## Eject (vendoring) and provenance

Eject writes one reference out in place (`refs::eject`; beatscriptls's code action, `beatscript expand`), under

```text
# from: gen@2 techno seed=42 section=drop part=bass -> z4EBG9j...
```

a comment, so the ejected program's canonical text and CID are those of the same notes typed by hand (tested), while the line
keeps the call and its pin. A section's or a track's own `# from ...` comment is replaced by it; the line goes above the
statement, indented as it is. The written-out text plays the same nets as the reference (tested for every place).

The program's JSON-LD (progld.rs, the `<beatscript-view>` block) says what it is based on: `isBasedOn`, a CreativeWork per call
or borrow its text makes and per part it vendored (read from its `# from:` lines), each with `resolves` (the pin),
`pinnedGenerator` (`seeded@2`) and `derivation` (`called`, `borrowed`, `vendored`); a recipe also says `expandsTo`, the CID of
its fully written-out program (`refs::expanded`: the same recipe, the same expanded CID; it plays the same). The terms are on
/schema (docs/SCHEMA.md).

**Curation (6670 item 4).** beatscript-serve's lineage index counts, per CID and per call, the public programs that call or borrow it
(`referencedBy`) and that vendored it (`vendoredFrom`), on each node's JSON-LD and through `Lineage::curation`. The store keeps a
program's canonical text, which has no comments, so a vendored count needs the save to keep the `# from:` lines in
`meta.based_on` (`["<call> -> <cid>"]`); the page's share/save does not send them yet (follow-up).

## Swing

**A call's program says its genre's swing.** The beats tables give each genre a `swing` from 0 to 100: jazz 60, blues 50, bossa 40,
lofi 35, funk and garage 30, reggae 25, house 20, dubstep, country and deephouse 15, dnb, trap and acid 10, electro 8, and 0 for
techno, trance, edm, ambient, metal, speedcore, synthwave and italo. seeded@1 and seeded@2 both print it as the program's
`swing` setting, on BeatScript's 0..1 scale: the table's value / 100, held to 0.6. So `gen@2 jazz` says `swing 0.6` and
`gen@2 house` says `swing 0.2`. A straight genre prints no `swing` line. `arrange from gen@2 ...` keeps the line unless the program
says its own `swing`. A part line, a section and a `let` never bring it, because it is a setting of the whole program, like the
tempo. `crates/beatscript-api/tests/generate.rs` holds every genre of every version to this.

## `seed@2`: seeds with their swing

```text
arrange from seed@2(jazz, 7)              the v1 track and `swing 0.6`
section intro from seed@2(jazz, 7)[1..8]  the v1 bars (a section brings no setting)
deck A: bass seed@2(jazz, 7).bass[1..4]   the same: a part, a let, bars, a structure as `seed(...)` takes them
```

**Grammar.** `seed = SEED "(" NAME "," INT [ "," structure ] ")" [ "." role ] [ "[" INT [ ".." INT ] "]" ]`; `SEED` is the
word `seed` or `seed@N`, touching its `(`, as `GEN` is `gen` or `gen@N` (grammar.ebnf, `recognize.rs`).

**What it lowers to.** Exactly what `seed(...)` (seeds v1) writes out, plus one line: `arrange from seed@2(...)` adds the
genre's `swing` when the program does not say its own. The value is `djlive::seed::swing_of`: the beats project's 0..100 / 100,
held to 0.6, which is the seeded generator's mapping. A straight genre writes no line, so `seed@2(techno, 42)` writes out the
same program as `seed(techno, 42)`, and the two expansions have one CID. The program as written keeps its version, so its CID is
its own. A reference resolves (`refs`, the pin) to the piece as a program of its own, with its swing, under the generator
`seeds@v2`. The expansion's `# from seed@2(...)` line ends `seeds v2`.

**What it refuses.**
- `seed@3(jazz, 7)`: "this BeatScript has seed (v1) and seed@2, not `seed@3`", help `seed@2(jazz, 7)`. It is one error, at the
  head. The head is quoted as written (`seed@99999999999`), and the help keeps the reference's own arguments.
- `arrange from seed@2 (jazz, 7)`: "`seed@2` touches its `(`", help `seed@2(jazz, 7)`. A bare `arrange from seed@2` is told the
  same (help `seed@2(jazz, 7)`), and so is a bare `seed (...)`.
- `#! seeds: v2`: "this BeatScript has seeds v1, not `v2`", help: write the version in the reference, `seed@2(techno, 42)`. A `#!`
  line is a comment and is outside the CID, so it pins nothing.
- Everything `seed(...)` refuses, the same way: an unknown genre (with the nearest names), bars outside the track, a part that is
  no role, a `let` that names no part.

**What it says (not an error).** A piece of a swung seed@2 (a section, a part line or a `let`) brings its notes and not its
swing, because the swing is a setting of the whole program. Where the program says no `swing` of its own, `check` gives an Info
diagnostic at the reference: "seed@2(jazz, 7)[1..8] brings its notes here, not its swing: `swing` is a setting of the whole
program, and only `arrange from` writes it", help `swing 0.6`. The editor's hover says the same by place. For the whole track it
says "writes `swing 0.6`".

**Decisions.**
- **The version went where the bug was (taste call, 2026-11-22).** Matt asked for "a NEW generator version, seeded@3 / gen@3",
  so that no old program changes. But the seeded generator (`gen@N`) already writes swing, as the section above says. The guard
  that never writes swing is seeds v1's (`djlive/src/seed.rs`, `whole`): it took a swing only in 0..1, while the beats project
  carries 0..100, so `arrange from seed(jazz, 7)` never said its swing, though "Start from a seed" said `arrange from` adds it.
  So the new version is `seed@2`, and there is no seeded@3.
- **In the text, as `gen@N`.** A `seed@2` reference is in the CID. A seeds v2 chosen by a `#! seeds:` line would give two kinds
  of music one CID. A bare `seed(...)` is v1 for good, as a bare `gen` is `gen@2` and a bare `macro` is `macro@1`, so every stored
  program keeps its text, its CID and its music.
- **`seed@1(...)` prints bare**, as `macro@1` does. The canonical text writes the version only when it is not 1, so
  `seed@1(techno, 42)` and `seed(techno, 42)` are one program.
- **Only the swing changes.** v2 is v1's writer with the swing guard fixed, and nothing else that would move a note, so the notes
  and the expanded CID of a straight genre are v1's (tested for all 19 genres).
- **No `#! seeds: v2` header in the expansion.** Its `# from seed@2(...) ... seeds v2` comment says the version for a reader. The
  `#! seeds:` line keeps meaning what a bare seed is.
- **The same mapping as the seeded generator.** The value / 100, held to 0.6. No genre is over 60, so the hold never acts today.
  It is there so the two generators agree.

**Tests.** `crates/djlive/tests/seed.rs`: `seed_at_2_writes_the_genres_swing_and_nothing_else` (every genre: the same lines as
v1 but the swing, the same notes, the swing line by the table), `seed_at_2_is_spelt_with_its_version_and_stands_where_a_seed_does`,
`an_unknown_seed_version_is_refused_with_the_ones_there_are`, `seeds_v1_write_no_swing`, and `seeds_v2_and_v1_are_pinned`
(`tests/fixtures/seed/seed-pins.txt`: the CIDs, unexpanded and written out, of seed@2 on swung and straight genres in each place,
and of the same bare seeds). Example: `examples/beatscript/swung-seed.beatscript`.

**Not done.**
- `seed@2` does not carry the beats project's `humanize`. BeatScript has no song-wide setting for it: `humanize()` is a pattern's
  velocity function.
- **Was: a `swing` line was not heard (measured 2026-11-22, fixed by the next Decision).** The review rendered programs with
  `beatscript render` at 48 kHz, groove on, 4 bars: a kick on the off-sixteenths only (`deck A: kick ~ x ~ x ...`), and `hihat x*16`.
  It took the onsets by threshold (`scratchpad/gen3-review/onsets.py`). At 120 BPM, `swing 0.6` delayed every off-step by
  exactly 18 samples (0.375 ms, 0.0030 of a sixteenth; the minimum equalled the maximum over 32 hits). `swing 1` delayed them by
  30 samples (0.625 ms, 0.0050). At 90 BPM, `swing 0.6` gave 24 samples (0.500 ms, again 0.0030 of a sixteenth). That is
  djrust's runner formula, `swing / 100 * sixteenth * 0.5`, applied to the unscaled 0..1 value that `DeckIr::envelope` handed it.
  beats' own jazz (60) is 0.3 of a sixteenth, so a program's `swing 0.6` was 1/100 of it.

**Decision (2026-11-22): a swing line is heard (taste call, one commit).** `DeckIr::envelope` (djlive/src/compile.rs) now hands
djrust the setting as its 0..100: `swing * 100`, rounded to 1e-9 so that 0.15 is 15. So `swing 0.6` is beats' 60, which is 0.3 of
a sixteenth: the off-step sounds `s / 2` of a sixteenth late (BEATSCRIPT.md, "Settings", defines it). That is the mapping gen@N and
seed@2 already write. The same probes, re-rendered from this tree (`scratchpad/gen3-measure/`):

| program | tempo | off-step delay | of a sixteenth |
|---|---|---|---|
| `kick ~ x ~ x ...`, `swing 0.6` | 120 | 1800 samples, 37.500 ms (min = max over 32 hits) | 0.3000 |
| `kick ~ x ~ x ...`, `swing 1` | 120 | 3000 samples, 62.500 ms | 0.5000 |
| `kick ~ x ~ x ...`, `swing 0.6` | 90 | 2400 samples, 50.000 ms | 0.3000 |
| `hihat x*16`, `swing 0.6` | 120 | mean 1799.3 samples (1785 to 1810), 37.49 ms | 0.2999 |
| `hihat x*16`, `swing 1` | 120 | mean 2999.6 samples, 62.49 ms | 0.4999 |

The even steps stay within a few samples of where they were (the onset threshold meets the tail of the step before). This
**changes the audio of every existing program that has a `swing` line**, hand-written or generated: the deep-house template's
`swing 0.12`, the forms' (`swing 0.08`, `0.18`), every gen@N of a swung genre. No program's text or CID moves. The nets and
envelope pins that moved are those of the programs with a swing line. In `crates/beatscript-api/tests/cid-pins.json` they are
`extra/swing-settings.beatscript`, `examples/beatscript/arith.beatscript`, `deep-house-60.beatscript`, `full-fathom-five.beatscript`, `rag-rondo.beatscript`,
`rondo-of-passacaglias.beatscript`, `such-stuff-as-dreams-122-reference.beatscript` and `swung-seed.beatscript` (their `nets`, `song_nets` and
`envelopes`; each `cid` is the same). The nets in `crates/beatscript-api/tests/golden/swing-settings.json` moved too. It is one commit,
so it can be dropped alone. Measured with `beatscript render` only: the page and the booth play the same envelope through the same
runner, but were not measured.

**Proposals.**
- Measure the setting in the page and the booth as well (the same envelope and runner; only `beatscript render` was measured).

## Offline, missing, CC0

- The generator is compiled into each host (beatscript-api installs it: `beatscript`, beatscript-serve, the page's wasm, beatscriptls). A host with none (djlive
  alone, the booth's djhost today) says `gen@2 is not here` at the call: hosts differ in whether it is there, never in what the
  music is. Nothing is fetched.
- A version a build no longer has is an error naming the ones it has. A stored program's CID a host lacks is the existing
  "is not here" error.
- Generated programs are CC0 by construction (a deterministic snapshot of a generator's choices is nobody's to license:
  docs/LICENSING.md; `generate` stores its output CC0). The provenance lines and `isBasedOn` credit them without licensing them.

## Not done

- The booth (djhost::beatscript_env) has no generator: `gen` calls check in `beatscript`, beatscript-serve, the page and the editor, not in djapp.
- A `gen` call's let takes the section's first stretch (an `evolve` in the generated section is not followed).
- `tempo=` was asked for as a name; seeded calls it `bpm`, and the call uses seeded's names (taste call).
