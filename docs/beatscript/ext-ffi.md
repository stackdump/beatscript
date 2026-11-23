# The engine side: calling the generator (`gen@v`), pinning, eject and provenance

Roadmaps 6668 (load seeded-pure generated tracks by reference), 6670 (BeatScript's FFI to beats.bitwrap), 6680 (recipe and
expanded CIDs). Built on `spec/gen-reference`. The short reference is BEATSCRIPT.md, "Two halves: structure and engine" and "Call the
generator: `gen@v`"; this note has the decisions and what is left.

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

**Which generator.** The seeded generator, `beatscript-api::generate`: the one beatscript.app's compose panel runs as "seeded, no
model", the MCP `generate` tool, `POST /api/generate`. It already shipped versioned (`#! generator: seeded@1`, then
`seeded@2` on 2026-11-22, which wrote chords as Roman numerals). `gen@N` is `seeded@N`, and its params are seeded's own
(`seed`, `bpm`, `key`, `length`), so nothing is invented. `seed(...)` is a different generator (beats.bitwrap.io's `compose`,
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
| `arrange from gen@2 ...` | the whole program, less the `tempo` and `key` lines the program says itself, under its `# from:` line |

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

## Offline, missing, CC0

- The generator is compiled into each host (beatscript-api installs it: beatscript, beatscript-serve, the page's wasm, beatscriptls). A host with none (djlive
  alone, the booth's djhost today) says `gen@2 is not here` at the call: hosts differ in whether it is there, never in what the
  music is. Nothing is fetched.
- A version a build no longer has is an error naming the ones it has. A stored program's CID a host lacks is the existing
  "is not here" error.
- Generated programs are CC0 by construction (a deterministic snapshot of a generator's choices is nobody's to license:
  docs/LICENSING.md; `generate` stores its output CC0). The provenance lines and `isBasedOn` credit them without licensing them.

## Not done

- The booth (djhost::beatscript_env) has no generator: `gen` calls check in beatscript, beatscript-serve, the page and the editor, not in djapp.
- A `gen` call's let takes the section's first stretch (an `evolve` in the generated section is not followed).
- `tempo=` was asked for as a name; seeded calls it `bpm`, and the call uses seeded's names (taste call).
