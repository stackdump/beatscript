# The DJ language: BeatScript for working DJs (design)

*Status: design, 2026-11-22 (val run 1f12e380), with the parts that need no new syntax being built. The name is a
placeholder until Matt picks one ("Taste calls" below); this page says **the DJ language** and **the DJ tier**. Every
construct marked *proposed* waits for his yes/no. project-datum: the design item `dj-language`, one proposed
`beatscript-roadmap` item per construct, and the taste-call list. It is written in the spelling of BeatScript 2, after the
btw to beatscript rename.*

BeatScript writes music as text. The DJ language **plays** it: the same language with what a working DJ reaches for at
the booth, namely tracks and loops as sources, a beat grid, cue points, stems, sync, the moves, and a set you can record
and play back. It is a **superset**: every BeatScript program is a DJ program as it stands, with the same CID and the same
sound. The DJ language adds constructs and never changes what a BeatScript line means.

```text
#! author: Likstatik
tempo 128
key am

use z4EBG9jQx7[17..32] as hook                   # proposed: a slice on `use` (run 16aa6899's `use SOURCE as NAME`)

section build 16 bars {
  deck A: kick x ~ x ~ | bass a1 ~ a1 ~
  deck B: load hook                              # proposed: a source name as the spec of the existing `load`
  at bar 8: deck B: play
  control: move loop_roll on B over 1 bar        # existing: a Perform move on the control deck
}
section drop 16 bars {
  deck A: kick x x x x
  deck B: mute vocals                            # proposed: a stem of the deck
  control: move bass_swap on B over 8 bars       # existing
  at bar 12: control: macro riser on A over 4 bars   # existing: macro@1, the DJ macro catalogue
}
arrange build drop
```

## What is built and what is proposed

| | built (no new syntax) | proposed (new syntax: Matt's yes/no) |
|---|---|---|
| sources | `sample`, `loop "..." as`, `use restricted` (existing); the engine renders a CC0 program by CID into a clip with an exact grid, sliced, and its four exact stems (tonight) | a slice on `use <cid>`; `stem(SOURCE, STEM)` for outside audio |
| scheduling | time words, scenes and `launch`, named arrangements as follow actions (existing) | nothing |
| sync | `deck B: sync`, `key detect` / `lock` / `status` (existing); every detection result names its analyzer, `beatgrid@1`, `beatgrid@2`, `key@1` (tonight) | a grid pin and overrides on a sample line |
| cues | `do deck hot_cue`, `do deck loop` (existing, sample decks); a beat jump on both deck kinds and loops on a net deck, in the engine, `deck jump` in the registry, the roll on a net deck a real loop (tonight) | `cue SOURCE.NAME at bar N`; `deck A: jump`, `deck A: loop` |
| stems | the four exact stems of a render (tonight); `mashup` stems, the `stemMute` deck action, `do deck role` (existing) | `deck A: mute vocals`, `unmute`, `solo` |
| moves | all six on the control deck; the render plays `filter_exit` and `echo_out` (existing); the other four in the offline render and `listen` where they can act offline (tonight) | `bass_swap` as a set transition |
| macros | `control: macro riser on A over 8 bars`, `macro@1`, lowering to `effects fire` (existing, run 16aa6899) | nothing |
| set recording | a recorded set written out as a `set { }` program plus a take that holds the action log (tonight) | outside sources in a set |
| authorship | `#! author:` with the `$DJ_AUTHOR` default (existing); forks keep the parent's author in lineage (tonight) | none |
| controllers | performed inputs `knob(N)` `pad(N)` `fader(A)` `held(key)`, controller profiles and presets, Mixxx import (existing) | `#! controller: <mapping cid>` |

## The superset rules

1. **Every BeatScript program is a DJ program, with the same CID.** A DJ host reads BeatScript text as it stands and
   computes the BeatScript CID for it: nothing is added, upgraded or re-spelled.
2. **One language, one CID: the DJ constructs are a tier of the grammar.** Suggested default (taste call 1): no version
   line of its own. The DJ productions are marked as the DJ tier in `grammar.ebnf`, so the drift test, the Grammar page and
   a separate generated section of the cheatsheet (BeatScript's stays inside its 30 KB) all come from the one grammar.
   `check` derives what a program needs and says it (`needs: booth`, and the same in the JSON-LD), and a host that lacks a
   construct says so at its line, the way `gen@2 is not here` reads at a host with no generator: hosts differ in what they
   can play, never in what the music is. A program's CID is computed one way whichever tier it uses, so the same music is
   never two CIDs, and nothing couples a DJ version to a BeatScript version. (The alternative, a version line `#! <name> 1`
   using BeatScript 2's version-line mechanism with a CID type of its own, is in taste call 1.)
3. **Two halves, as in BeatScript.** Structure stays canonical text: sources by content address, sections, the set.
   Everything that reaches into the engine is an engine call through the FFI half (`docs/beatscript/ext-ffi.md`): a move,
   a macro, a verb of the control registry, a render, a stem split, a beat grid. The text records the call, never its
   result, so a call whose result changes the music is pinned by a version and resolved before lowering, as `gen@N` is.
4. **Public and booth are structural, and stricter for the DJ tier.** beatscript.app already refuses to store any program
   that uses an asset. The DJ tier goes further, as a new rule: any DJ construct that reads outside audio as a deck source
   (loading, cueing, jumping, gridding, keying or splitting a sample or a library loop) needs `use restricted`, whatever the
   asset's licence tag, and such a program is never shared or stored. BeatScript's own `sample ... inst=` tiers stay as
   they are (rule 1). A restricted program goes to the owner's paired devices only, sealed, through Push-private (which
   carries the text and stores nothing).
5. **Outside audio is pinned by its bytes.** A source is named by `sha256:<hex>` in the canonical text, never by a path or
   a file name; the booth keeps the path on its own machine. The same text with the same files is the same program; a
   machine without the file says so at the line ("missing asset"), never plays something else quietly.
6. **Every engine computation that changes the music is pinned by version.** Detection of outside audio: `beatgrid@1`
   (the DSP grid), `beatgrid@2` (the Beat This neural grid, its model pinned by SHA-256), `key@1` (the ensemble key
   profile), `loud@1` (loudness, EBU R128, for auto-gain), `stems@1` (the HT-Demucs split). Playback: `stretch@1` (the time
   stretch, key lock on, when a source plays away from its own tempo) and `render@1` (the offline renderer's output, the
   same samples on every host, held by the cross-host render tests; a render never includes the booth's live-only units).
   A version is a frozen algorithm plus pinned model bytes, never edited: a better detector is `@2`. A bare spelling means
   the version current on the day its construct ships, for good (the `macro` rule: the default is not printed). Results
   are cached by (asset hash, analyzer and version), and each resolved result goes into the program's resolution manifest
   and its `expanded_cid` the way the `gen@N` pins do, so a host that would resolve differently shows up mechanically.
   Neural analyzers must be shown deterministic across Mac arm64, Linux x86 and Android before a version is frozen.
7. **Programs need no detection.** A BeatScript program by CID is the ideal DJ source: deterministic, and it says its own
   tempo, key, sections and parts. Its beat grid, its cue points (its sections) and its stems (its parts by role) are exact
   and CC0 when the program is. Whether a deck plays it as nets (exact at any tempo) or as its render (a sample deck) is the
   engine's business, not the text's: one source form, `use <cid>`, covers both.
8. **The stems rule.** Meta's HT-Demucs weights are for research and personal use only (project-datum 6697). So `stems@1`
   runs at the booth only, opt-in, under `use restricted`, and the weights are never bundled, committed or shipped in an app
   or the browser. Public stems of outside audio wait for our own separation weights, trained on generated programs whose
   exact stems come free (6697's roadmap). A program's stems need no weights at all (rule 7).
9. **No CID moves.** Every BeatScript program keeps its canonical text and CID (the CID pin tests hold it).

## Layers: what is public, what is the booth's

"Public" means the program may be shared and stored on beatscript.app.

| construct | public | booth only (`use restricted`) |
|---|---|---|
| a program by CID: its sections, parts, slices, render and stems | yes (`play <cid>` in a set, `<cid>.drop`, `use <cid> as` when it lands) | |
| a sample or a library loop as a deck source, its grid, key, loudness or cues | | yes |
| stems of outside audio (`stems@1`) | | yes, opt-in |
| moves, macros, the control deck, scenes, `launch` | yes | |
| a recorded set of programs | yes | |
| a recorded set that played outside audio | | yes |
| a controller mapping | data, not text | a Mixxx-derived mapping (a translation of a GPL file) |

## Sources: clips and loops

A **source** is audio a deck can hold: the DJ's track, loop or one-shot.

**Existing.** `sample NAME from "sha256:..." license L else VOICE` declares outside audio by hash with a licence tag
(`cc0`, `cc-by`, `all-rights-reserved`, `restricted-private`) and a stand-in voice for the public fork; `loop "<name or
hash>" as NAME else VOICE` names a library loop (its tier is always the library's, shareable as a hash reference, never
CC0); `use restricted` makes the program private, and `check` requires it for a `restricted-private` asset (BEATSCRIPT.md,
"Restricted sounds"). The engine has no sampler for a user file on a net yet, so a part that names a sample plays its
stand-in with a warning. A program's sections and parts are borrowed by CID (`section hook from <cid>.drop`, `bass
<cid>.drop.bass`), and run 16aa6899 is landing `use SOURCE as NAME` (a stored program, section or part by CID, a theme, a
phrasebook line, the genre templates).

**Built tonight (engine, no syntax).** The engine renders a public CC0 program by CID into a clip: stereo PCM with an
exact grid from the program's tempo map (bar 1 at frame 0), sliced by bars from 1 inclusive (`[17..24]`, the slice
`seed(techno, 42).bass[17..24]` already uses), its audio SHA-256 recorded, made by `render@1`. It refuses a program that is
not CC0 and public, or that uses restricted assets. It also renders the program's four DJ stems exactly (below). The
registry's `deck load` documents `spec=render:<cid>` and `render:<cid>[a..b]` (`djhost::clip_source`); the booth's handler
that loads one on a sample deck is still to be wired.

**Proposed.** Programs need no new source word (rule 7): a slice on `use`, and a source name as the spec of `load`
(taste call 3). Outside audio gains one function, `stem(SOURCE, STEM)` (taste call 4):

```text
use z4EBG9jQx7[17..32] as hook                   # bars 17 to 32 of a program
use z4EBG9jQx7.drop as drop                      # one section (`use` with a section, as 16aa6899 lands it)
deck B: load hook                                # a net deck plays its nets, a sample deck its render

use restricted
sample tune from "sha256:9f2c..." license restricted-private else bright-pluck
use stem(tune, vocals) as tune_acap              # stems@1: booth only
deck A: load tune_acap
```

A declared source whose name is also a `load` spec (a genre name, a loop set) or a saved program's name is an error at the
declaration, so `load hook` and `play hook` are never ambiguous. A program that uses another program's render or stems
`isBasedOn` its CID, with `derivation: rendered`. Roadmap 6708 ("sample from render", filed by run 16aa6899 for both runs)
folds into this: its source is `use <cid>`, and its one-shot use is the existing `inst=` once the sampler exists.

**Prep travels with the track.** A track's prep (its source line, its grid pin, its cue points) is written once in a small
program and brought in by `import <cid>`, so a set does not repeat it; this needs `import` to carry source and cue
declarations as it carries lets and defs (part of taste call 3).

## Scheduling

Everything a DJ schedules has a spelling already. The DJ language adds no scheduling word.

| want | write (existing) |
|---|---|
| at bar N | `at bar 32: deck B: play`, `at bar 16 beat 3: ...` |
| loop N bars | a section of N bars launched as a scene loops; or a deck loop (`do deck loop deck=A op=beats beats=16`; proposed sugar `deck A: loop 16 beats`) |
| quantized launch | `launch drop on pad(2) next bar` / `next phrase 16`; one clip's quantum `do clips quantize`, the clip row's `do clips row_quantize beats=4` |
| on the phrase | `on phrase:`, `on next phrase 16:`, `before drop 4 bars:`, `after drop:` |
| follow actions | named arrangements: `arrange set: drop wubs loop 4` then on to the next, `then NAME`, `end`; a scene is a section or a named arrangement |
| a pad held down | `when pad(5)` on a control line |

## Sync: beat grid, tempo, phase, key, level

**Existing.** `deck B: sync` (tempo and phase to the other deck, aligned to the beat or the bar), a timed `tempo` on the
control deck, `key detect deck=A`, `key lock deck=A on=true`, `key status` (each deck's Camelot key and the pair's
`compat`: same, relative, adjacent, diagonal, boost, parallel or clash, with a hint and a suggested transpose when they do
not mix), `key compat a=8A b=3B` for two keys, the Perform window's harmonic hint (same, fifth, relative, clash), and a
set's `check`, which gives each track's Camelot key and warns where neighbours clash or their tempos are more than 8% apart.

**Built tonight.** Every analysis result names its analyzer and version (`beatgrid@1` DSP, `beatgrid@2:<model sha256>` the
neural grid), and the analysis cache (schema `djrust/beatgrid/v2`) is keyed by it, so a changed detector misses instead of
returning an old grid. `key@1` is defined beside them; the public analysis does not compute keys yet (the booth's key
detection does).

**Proposed (taste call 5).** A sample line takes its grid analyzer and the numbers that override detection:

```text
sample tune from "sha256:9f2c..." license restricted-private else bright-pluck grid=beatgrid@2
sample dub from "sha256:41aa..." license restricted-private else pluck bpm=124 offset=183ms key=am gain=-3db
```

`grid=` is the only pin parameter (`key@N`, `loud@N` and `stems@N` pins arrive when an `@2` exists); `key=` only ever
takes a key, `bpm=` and `offset=` a grid, `gain=` a level. The order is: the text, then the file's own tags (BPM and key
tags in the file are pinned by its hash), then the analyzer. `check` says what each resolved to at the line (`tune:
beatgrid@2 -> 126.0 bpm, bar 1 at 0.183s; key@1 -> 8A; loud@1 -> -9.1 LUFS`), and gives harmonic hints in the words a set's
check uses: each source's Camelot key against the program's `key` and against what it is mixed with. A clash is a warning,
never an error.

**Which grid is the frozen default** is the second half of taste call 5: `beatgrid@2` scores far better (beat F1 88.7
against 68.9 for the current folded DSP grid on GTZAN, by the validation branch's measurements), but its determinism across
runs, thread counts and chips is not proven. Suggested: ship the construct with `beatgrid@2` as its bare default once a
test shows the same bytes give the same grid on the Mac, Linux and Android, with one inference thread if that is what it
takes; until then, the construct waits.

**Decided by the language, not a knob:** auto-gain (a source is played at `loud@1`'s level unless `gain=` says), the
crossfader curve for values the text writes (the booth's own curve applies only to the performed fader, and the take
records it), and loops written in text are slip loops (`for` puts the track back where the bars say it would be).

## Cues: cue points, hot cues, loop rolls, beat jumps

**Existing.** The deck's own verbs: `do deck hot_cue deck=A slot=3`, `clear_cue`, `cue`, `loop op=in|out|halve|double|exit|beats`,
`seek seconds=...`, `section index=...`; hot cues work on both deck kinds, loops now on both. `move loop_roll on A over 1 bar`
is the roll build (a 4-beat loop halved every `bars` to half a beat, released on the line: a sample deck's loop, or now a
net deck's), and `trick on kind=stutter` a beat repeat.

**Built tonight (engine, and the booth host uses it).** A beat jump on both deck kinds, keeping the phase, recorded in the
action log; and loops on a net deck, so a generated track can be held and rolled like a sample (the limits are in the
engine's notes). The engine pin moved to that build. The registry has `do deck jump deck=A beats=-8`, `deck loop` says it
works on a net deck, and the loop roll on a net deck is now the net's own loop, live and in the offline render, its halves
and its release in the action log (docs/PERFORM.md). The booth's socket handler for `jump` is the booth's. Known: a roll
whose release falls on the next section's line comes out about a beat out of phase (that section's net reaches the deck two
beats early and ends the loop); docs/PERFORM.md says so, and the fix is an engine call.

**Proposed (taste calls 6 and 7).** A cue point is a named position in a source. A program's cue points are its sections,
so `hook.drop` needs no declaration. A sample declares them:

```text
cue tune.intro at bar 1
cue tune.drop at bar 33 beat 2
at bar 16: deck A: jump tune.drop                # a cue jump, landing on the beat
deck A: jump -8 beats                            # a beat jump (also `16 bars`)
at bar 24 for 4 bars: deck A: loop 4 beats       # a slip loop: `for` lets go
```

`jump` and `loop` are deck actions beside `play`, `stop` and `sync`, lowering to the deck verbs above and tonight's beat
jump. **Hot cues are not syntax:** a pad that jumps a deck to a cue is a binding in the controller mapping (pad 3 -> `deck
jump cue=tune.drop`), and a program can name its cue points for the mapping to use. `launch` keeps one meaning, scenes.

## The Perform moves, as calls

All of them exist, on the control deck and off it:

| the move | write | lowers to | what it does |
|---|---|---|---|
| bass swap | `control: move bass_swap on B over 16 bars` | `moves arm` | the incoming deck's lows cut at once, swapped on the phrase line |
| phrase-synced transition | `control: move phrase_sync on B over 8 bars` | `moves arm` | the seed steps a bar early so the new track lands on the line (`do moves arm kind=phrase_sync deck=B bars=8 do=cut` cuts the crossfader to the deck on the line instead) |
| filter-sweep exit | `control: move filter_exit on A over 4 bars` | `moves arm` | a high-pass rides out over the bars, then the deck is muted on the downbeat |
| echo out | `control: move echo_out on A over 2 bars` | `moves arm` | the wash on the one: the dry goes, the room rings, the deck is muted after the tail |
| loop-roll build | `control: move loop_roll on B over 1 bar` | `moves arm` | the deck's loop, 4, 2, 1, half a beat, released on the drop (a sample deck or a net deck; a sample with no grid: the stutter's repeats) |
| vocoder handoff | `control: move vocoder_handoff on B over 8 bars` | `moves arm` | the vocoder's modulator and carrier swap on the line |
| harmonic-mix hint | `key status` (a read; `check` says it for sources) | `key` | the pair's compat and the transpose that would fix it |
| swap the pair | `on next phrase: control: swap pairs with filter_exit` | `autodj set` + `autodj next` | the loop-set pair steps on, with a move across the handover |

**Built tonight.** The offline render already played `filter_exit` and `echo_out` from the program's control net; the other
moves now play there too where they can act offline, so `render`, `listen`, the MCP tools and the booth hear the same moves.
A move that cannot act without the booth (a vocoder handoff needs a live vocoder; a phrase sync's seed step needs a seed
deck) is named in the render's notes, never faked.

**Engine follow-up, not syntax:** an echo out through a real delay (the strip's mute comes after the inserts today). The roll
on a net deck now uses the net-deck loop instead of the stutter.

## Macros

The DJ macro catalogue (39 effects: risers, sweeps, cuts, breakdowns, tricks, pan and shape) is built (run 16aa6899,
`docs/beatscript/ext-control.md`): `control: macro riser on A over 8 bars`, pinned by the frozen `macro@1` table (the
canonical text writes `macro@2` only when the version is not 1), lowering to the registry's `effects fire id=ID target=DECK
bars=N`, sent a beat early so it starts on the bar its line names, heard offline and named by `listen`. The DJ language
uses it as it is and adds nothing beside it. A move stays a `move`: a move is a schedule over the strip (when to cut, when to
mute, when to let go), a macro one effect for a length; folding them would lose the move's put-back rules.

## Stems: solo, mute, swap

**One vocabulary: the four DJ stems,** `drums`, `bass`, `vocals` and `other`, where `other` is everything that is not one of
the first three (a net deck's harmony, melody and any other role). On a net deck a stem is its roles: `drums` the drum roles,
`bass` the bass, `vocals` the sung parts.

**Existing.** A generated track splits into exact stems by its nets (`djengine::stems::NetStems`). The `mashup` noun assigns
stems to slots; the deck action `stemMute` mutes one; `do deck role deck=A role=bass on=false` turns a net deck's role off.
The outside-audio splitter is a stub in the public engine.

**Built tonight.** The four DJ stems of a program's render, exact, and CC0 when the program is: a stem is rendered with the
mix's own limiter gains replayed, as `beatscript export` makes its stems, so the four add up to the mix within float rounding.
A riser into a drop, a stinger and the control nets are in no stem.

**Proposed (taste call 8).**

```text
deck A: mute vocals                              # the existing `mute`, with a stem: `stemMute`, or the roles on a net deck
deck A: solo drums                               # the other three muted
at bar 32 for 8 bars: deck B: mute bass          # `for` puts it back; `unmute bass` too
control: move bass_swap on B over 8 bars         # swaps the bass stems when both decks have them, the lows otherwise (engine)
```

A swap of another stem is `do moves arm kind=stem_swap deck=B stem=vocals bars=8`, a move kind the scheduler would gain, with
no new grammar. Outside audio's stems are `stems@1` (rule 8): a prepare step at the booth (HT-Demucs runs at about 1.3x real
time on eight CPU threads, so never while you play), cached by (asset hash, `stems@1`), and pinned to one configuration
(single-threaded inference, so the split is the same every time). Effects on one stem: roadmap, not syntax.

## Set recording

**Existing.** The booth's `record` noun records a performed set: the master to a WAV, a timestamped tracklist of CIDs, the
action log passage (`djrust.arrangement/1`: every human-level action at its beat), signed, content-addressed. A
**performance** (`djrust.performance`) is a take of a program's performed inputs, named by the program's CID and never in
its text. A **set** program (`set { play <cid> ... mix N bars with ... }`) is the language's programs of programs.

**Built tonight.** A recorded set is written out as a program and a take:

- **the program**, a BeatScript `set { }` in existing syntax: one `play <cid>` per track that was a program (with `until bar
  N` where the next track cut it short), one `mix N bars with T [, tempo A -> B]` per transition, header lines for the name
  and the author (`#! author:` from the recording, else the booth's default), CC0 and public unless a track was outside
  audio, which makes it `use restricted` and private. A move with no set spelling (a bass swap, a loop roll, a phrase sync,
  a vocoder handoff) is written as the nearest transition with a `# performed:` comment above it; an entry that is not a
  program is a comment line that keeps the timeline honest.
- **the take** (`djrust.set-take`): the action log, the tracklist and the recording's CID, named by the program's CID
  exactly as a performance names its program, with a CID of its own. The program is the readable score; the take is the
  exact replay. `beatscript set-program <recording>` writes both; the booth's `record program id=...` is the same call
  (its handler is the booth's, still to be wired). The set is BeatScript 2. Two things the booth must add for real recordings:
  a `record mark kind=track cid=<program cid>` when it evaluates or loads a program (today a live-coded track appears only as
  net loads, written as a gap comment), and the configured author in the recording (else `$DJ_AUTHOR` is used).

**Proposed (taste call 9).** A set may declare outside sources and `play` them by name (a booth set instead of comments),
and `mix N bars with bass_swap`, the one move a set's check can prove (a set mix already swaps a role both tracks share at
its middle). `loop_roll`, `phrase_sync` and `vocoder_handoff` stay `# performed:` lines plus the take. Not proposed: the
take's actions as timed control lines inside a set; a performance is not the program, here or in BeatScript.

## Authorship

Matt's DJ name is attached by default. No new field: BeatScript's own header and rights carry it.

- **`#! author: Likstatik`** is the existing header line. It is out of the program CID like every header line, and goes into
  the rights (the object CID), the program's JSON-LD `author` and the store's credits.
- **The default** is the booth's: every new program, recorded set and fork made at the booth gets the configured author
  without typing it, through the existing `$DJ_AUTHOR` default, which the booth sets from its own config (run 487306e0). A
  program that names its author keeps it.
- **Forks** keep the parent's author in lineage and name the forker: built tonight, a fork's rights hold a credit for the
  parent with its author for every licence (it was kept only for a CC BY parent), and `forked-from` as before.
- **share** carries the author into the stored rights' credits; **Push-private** carries the header text to the owner's
  phone and stores nothing.
- **Signing**: when the owner's key signs a program, the name and the key go together. The site makes no naming guarantee;
  the signed data is the proof (to be said on beatscript.app/authorship, which today covers CC0 and AI authorship).
- **Public BeatScript too** (proposed, not syntax): a signed-in beatscript.app account gets a "default author name" setting
  that the editor and the MCP `share` tool apply when a program names none.

## Controller mappings as data

**Existing.** A program reads performed inputs only by abstract name, `knob(N)`, `pad(N)`, `fader(A)`, `held(key)`,
`held(c4)`, so it checks, saves and shares the same on every machine. The booth's controller profiles and per-device presets
are JSON data, for the Hercules DJControl Inpulse, the Akai MPK Mini, the PD705 pad kits, the APC mini and the TD-3, with a
bindings window and a press-to-test map. Mixxx mappings are read from the user's own `*.midi.xml` files at run time (Mixxx is
GPL-2+, so nothing is vendored) and become devices; their scripted controls are not translated yet.

**Proposed (taste call 10).** `#! controller: <mapping cid>` in a program's header (out of the CID, like `#! genre:`) says
which mapping it was written for, and the booth offers to load it. Not syntax, so the factory's to build: a mapping is a
content-addressed data object (its CID over its canonical JSON), a take records the mapping it was played with, and `check`,
given a mapping, says which physical control each input is (`knob(3)`: the Inpulse's filter knob, deck A). A Mixxx-derived
mapping is a translation of a GPL file: it stays at the booth unless its licence allows sharing.

## What stays in the booth's hands, not the text

So these do not come back as syntax: the headphone cue (PFL), the pitch fader, its range and nudge, live loop halve and double,
the slip and quantize toggles, browsing the library and crates (crates are tags plus the store's search; the set is the
playlist). A program can still set a deck's tempo and lock its key (`tempo`, `key lock`); the hand on the fader is the take's.

## MCP tools and the booth Code view

No new tool (the sprawl check): every DJ capability is a verb of a noun the registry already has, or a program line.

- **Public MCP** (beatscript.app): `check`, `listen`, `render`, `fmt`, `eval` and `share` take a public DJ program; `check`
  says what it needs. `listen` hears the moves the render plays.
- **The booth MCP** (`djapp_*`, one tool per noun): `djapp_record` gains `program`; `djapp_deck` gains `load
  spec=render:<cid>[a..b]` and the beat jump; `djapp_analysis` answers the analyzer of every result. The handlers are the
  booth's.
- **The booth Code view** (the full-screen code window): a DJ program plays there like any program, with the set's follow
  (the playhead on the `play` or `mix` line); a finished recording opens as its program, with its take beside it to replay.

## Sprawl check

- **No new source word for programs:** `use <cid>` with a slice; render or nets is the engine's choice. Outside audio adds one
  function, `stem(...)`.
- **No new scheduling vocabulary**, and `launch` keeps one meaning (scenes); hot cues are bindings.
- **No new transport nouns:** `jump`, `loop`, `mute STEM`, `solo STEM` are actions of the deck that already has `play`, `stop`,
  `sync`, `mute`, each lowering to an existing or tonight's deck verb.
- **No new MCP tools and no new registry nouns:** `record program`, `deck jump`, `deck load spec=render:` are verbs and params of
  existing nouns.
- **One meaning per parameter** (`key=` is a key; `grid=` the only pin), **one stem vocabulary**, **one slice spelling**
  (`[a..b]`), **one pin spelling** (`name@N`), **one content address** per kind (`sha256:` for outside bytes, the CID for
  programs).
- **A gap found on the way** (a bug in BeatScript, recorded separately, not fixed tonight because it changes which programs
  check): the `do` surface can name outside audio past the rights check (`do deck file path=...` checks only the parameter's
  type). The fix is one generic rule: the registry marks its asset-bearing parameters, and `check` sends them through the same
  asset resolution as `sample` (a path is refused in a program; a `render:` spec gets the CC0 check).

## Taste calls (batched, each with a suggested default)

Syntax, in the order to decide them:

1. **A version line for the superset?** *Suggested: no* (rule 2: a tier of the one grammar, `check` derives `needs: booth`).
   The alternative is a version line `#! <name> 1` with a CID type of its own; it costs two CIDs for the same music and couples
   the DJ version to BeatScript's.
2. **The name.** With no version line it labels the tier and the docs; with one, it is also the header and the extension.
   (a) **Selecta**: `#! selecta 1`, `.selecta`: the sound system's word for the one who chooses the records ("the art is in the
   choosing"); no clash with a word of the language or a Workspace project. (b) **SpinScript**: `.spin`: a sibling of
   BeatScript, but SPIN is a well-known model checker in pflow's own field, `.spin` is the Propeller language's extension, and
   `spin_back` is a macro id. (c) **BoothScript**: `.booth`: but "booth" already names the private tier, so "a booth program"
   would mean two things. *Suggested: Selecta.*
3. **Program sources:** a slice on `use` (`use <cid>[a..b] as NAME`), a source name as the spec of `load`, and `import` carrying
   source and cue declarations (a track's prep imported by CID). *Suggested: yes.*
4. **`stem(SOURCE, drums|bass|vocals|other)`** for outside audio (`stems@1`, booth only). *Suggested: yes.*
5. **The grid pin and overrides on a sample line** (`grid=beatgrid@N`, `bpm=`, `offset=`, `key=`, `gain=`), shipped with
   `beatgrid@2` as the frozen bare default once it is proven deterministic. *Suggested: yes, and wait for the proof.*
6. **Cue points:** `cue SOURCE.NAME at bar N [beat B]`, a program's sections being its cues already. *Suggested: yes.*
7. **Deck actions** `jump CUE | N beats | N bars` and `loop N beats | off`. *Suggested: yes.*
8. **Per-stem deck actions** `mute`, `unmute`, `solo` with a stem. *Suggested: yes.*
9. **Sets:** outside sources declared and played by name, and `mix N bars with bass_swap`. *Suggested: yes to both.*
10. **`#! controller: <mapping cid>`.** *Suggested: yes.*

Policy, not syntax:

- Public stems of outside audio stay blocked until our own weights exist (6697). *Suggested: keep.*
- The DJ tier's stricter rule 4 (every outside-audio deck source needs `use restricted`, whatever its tag). *Suggested: keep;
  it is the rule set for this design.*
- The beatscript.app account "default author name" setting. *Suggested: build it after the rename.*

## Not done (after tonight)

- Every proposed construct (they wait for the yes/no), and the grammar, cheatsheet, ontology term and JSON-LD for each as it
  is approved: a construct is not done until it is in all four.
- The sampler for a user's file on a net (a sample still plays its stand-in), and a program's render played as a source in the
  browser.
- `stems@1` wired into the booth (the public engine's splitter is a stub; separation runs as a separate tool).
- The booth handlers for `record program`, `deck jump` and `deck load spec=render:` (the booth run's side).
- The `do` rights gap above; the beatscript.app default-author setting; named hot cues with colours; decks 3 and 4; Mixxx
  scripted controls.
