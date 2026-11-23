# The control deck (`control:`) and `macro`

*Roadmap 6671 (the control deck) and 6670 item 2 (`macro`), both built. Code: `crates/djlive/src/parse.rs`
(`control_deck_body`, `control_head_next`, `control_head_at`, `macro_call`), `ast.rs` (`Line::control`, `Action::Macro`),
`print.rs`, `expand.rs` (the head dropped before lowering, the untimed tempo refused), `macros.rs` (the `macro@1` table, the
call's lead), `compile.rs` (`macro_call`), `nets.rs` (`fire` / `release`), `highlight.rs`; the engine verb
`crates/djhost/src/fx_view.rs` (`fire`) and the registry's `effects fire`; the grammar `crates/djlive/grammar.ebnf` (`headed`,
`engine`, `MACRO`); tests `crates/djlive/tests/control_deck.rs`, `crates/djlive/tests/macros.rs`, `crates/djhost/tests/macro_render.rs`.
The rule it follows is BEATSCRIPT.md's "Two halves: structure and engine".*

A program has two halves, its structure and its engine calls. The control deck is where the engine calls go: just another
deck, one that only takes engine commands, written beside `deck A:`, `deck B:` and `master:` and sequenced by sections,
repeats, defs and time words like any line.

```text
section drop 16 bars {
  deck A: kick x x x x | bass c2 ~ c2 ~
  deck B: arp arp(am7, up, 8)
  master: filter lp 400 -> 8k over 8 bars
  control: move echo_out on B
  at bar 12: control: swap pairs with filter_exit
}
```

## Grammar

```ebnf
headed      = "deck" deck ":" parts [ guard ]
            | [ timing ":" ] [ target ":" ] control [ cguard ]
            | [ timing ":" ] "control" ":" ( engine | do | duck | mix ) [ cguard ]   (* the control deck *)
            | [ timing ":" ] ( invoke | duck | mix ) [ cguard ] ;
control     = action | engine | do | sing ;
action      = "filter" ... | ( "level" | "low" | "mid" | "high" ) change | "load" SPEC | "play" | "stop" | ... ;
engine      = "crossfader" change
            | "tempo" tempo
            | "swap" "pairs" [ "with" MOVE ]
            | "move" MOVE "on" deck [ span ]
            | "load" "pair" SPEC [ "group" INT ]
            | "queue" ( "play" | "pause" | "stop" | "next" | "loop" | "jump" INT | "add" WORD )
            | MACRO NAME ( "on" deck [ span ] | span "on" deck ) ;   (* MACRO: macro, macro@1 *)
```

`engine` is what `action` used to hold for the lines that take no target; it moved out of `action` into a production of its
own so the grammar can say what the control deck takes, and `control` (a line's body after an optional target) is `action |
engine | do | sing` as before, the same sentences. The grammar is exact about the control deck: the parser refuses nothing the
`engine | do | duck | mix` alternative writes (the drift test, `tests/grammar.rs`), and what it refuses is not in it. (The check
refuses one thing more, an untimed `control: tempo 140`: see Decisions.)

The head goes where a target goes: after the time word and its colon, before the call. `control: at bar 4: ...` is an error
(the time word comes first: `at bar 4: control: ...`), and so is a second head (`deck A: control: ...`, `control: master: ...`).

## What it takes, and what it lowers to

| on the control deck | is the line with no head | registry calls (BEATSCRIPT.md "Control") |
|---|---|---|
| `control: move echo_out on B over 4 bars` | `move echo_out on B over 4 bars` | `moves {action: arm, kind: echo_out, deck: B, bars: 4}` |
| `control: swap pairs with filter_exit` | `swap pairs with filter_exit` | `autodj {action: set, style: filter}`, `autodj {action: next}` |
| `control: load pair dnb:2602 group 2` | `load pair dnb:2602 group 2` | `loopsets {action: load_pair, ...}` |
| `at bar 32: control: queue next` | `at bar 32: queue next` | `queue {action: next}` |
| `control: crossfader 0 -> 1 over 8 bars exp` | `crossfader 0 -> 1 over 8 bars exp` | `mixer {crossfader: v}`, ramped |
| `at bar 16: control: tempo 128 -> 140 over 4 bars` | `at bar 16: tempo 128 -> 140 over 4 bars` | none: an event of the tempo map |
| `control: duck B under A.kick 6db` | `duck B under A.kick 6db` | `trick {action: on, kind: pump, ...}` |
| `control: mix { A level 0.7 }` | `mix { A level 0.7 }` | `mixer {deck: A, volume: 0.7}`, put back at the section's end |
| `control: do vocoder set mix=0 -> 1 over 8 bars` | `do vocoder set mix=0 -> 1 over 8 bars` | `vocoder {action: set, mix: v}`, ramped |
| `every 16 bars: control: freeze phrase bars=2` | `every 16 bars: freeze phrase bars=2` | `freeze {action: phrase, bars: 2}` |
| `at bar 24: control: macro cathedral on B over 2 bars` | `at bar 24: macro cathedral on B over 2 bars` | `effects {action: fire, target: B, id: cathedral, bars: 2}`, sent at bar 23 beat 3 (below) |

**Lowering is the line with no head.** The parser reads `control:` into `Line::control` (the head's span) and leaves the line
with no target; `expand.rs` drops the head when it lowers the line, so everything after it (the compiler, the control net,
the tempo map, the runtime) sees the line with no head. Each check of the call is the line's own (`control: move echo_out`
needs its deck, `control: move bass_swap on B over 2 beats` takes whole bars), and so is each time word's meaning, `for N
bars`, a guard (`when $i % 2 == 1`, `when knob(1) > 0.5`) and the `#2` of a second identical line.

The control's **key** is the canonical text of the line with no head (`move echo_out on B`). The runtime matches a program's
controls to the running ones by key, so an edit that moves a line onto the control deck (or off it) re-fires nothing. The one
visible trace of that is in the words the runtime says: `bar 12: swap pairs with filter_exit` names the line without its
head.

**The canonical text is its own.** `control: move echo_out on B` prints with its head, one space after the colon (and after
the time word's: `at bar 12: control: swap pairs with filter_exit`), so the two spellings are two texts and two program CIDs
of one lowering: the same runtime controls, deck nets, control-net events and tempo map
(`tests/control_deck.rs` holds sixteen such pairs to each of these).

## What it refuses

| line | error | help |
|---|---|---|
| `control: kick x ~ x ~` (a part, a `stack(...)`, a note) | the control deck holds engine calls, never parts or notes | a part goes on a note deck: `deck A: kick x ~ x ~` |
| `control: filter lp 400`, `delay`, `reverb`, `crush`, `drive`, `resonance`, `level`, `low` / `mid` / `high` | turns a deck's or the master's strip, so it is not a line of the control deck | a strip's action goes on its deck or the master: `deck A:`, `deck B:` or `master:` |
| `control: play`, `stop`, `sync`, `mute`, `unmute`, `load techno:42` | is a deck's | `deck A: play`, `deck B: load techno:42` |
| `control: sing "..." on melody` | `sing` sings a deck's part, so it is no engine call | `deck B: sing ...`, or with no head |
| `control: drop(A, 8)` | a def's call names its decks itself | with no head; inside the def, `control:` goes on its engine lines |
| `control: tempo 140` (no time word) | a tempo with no time word is a setting, structure | `tempo 140`; on the control deck a tempo is a change at a place |
| `control: key am`, `control: swing 0.1` | a setting of the program, structure | `key am` (`control: key detect deck=A` is the registry's `key`) |
| `deck A: control: ...`, `control: deck A: ...` | a line has one head | |
| `control: at bar 4: ...` | a time word comes before the head | `at bar 4: control: ...` |
| `control: mvoe echo_out on B`, `control: macor riser on A` | not an engine call the control deck takes | did you mean `move`? / `macro`? |

A note deck keeps the rules it had: `deck A: move ...`, `master: swap pairs`, `deck B: crossfader 1` still "take no target",
and `filter lp 400` with no head still "needs a target".

## Decisions

- **`control` is no reserved word.** It is the head only before a colon, at the head of a line or after a time word's colon
  (the parser reads it where a target goes; `parse::control_head_at` is the same test for the scalar-let prepass and the
  highlighter, which see tokens, not lines). No example and no saved program in the repo uses `control` as a name, but
  reserving it would buy nothing: `control` and a colon in that place could not be anything else. So a let, a def or a section may still be called `control` (`section control 4 bars { ... }`,
  `after control: control: queue next`, `control(A)` calls a def of that name), and a scalar `let control = 4` is put in place
  where `control` is a name (`at bar control:`) and not in the head.
- **What the control deck takes** is every line that takes no target today and is an engine call: the booth's `move`, `swap
  pairs`, `load pair`, `queue`, the `crossfader`, a timed `tempo`, the registry (`do` and the sugar nouns), and `duck` and
  `mix`, which name their decks themselves and are engine calls (BEATSCRIPT.md's "Two halves" table puts them with the voice and the
  mix). Left out: `sing` (it writes a sung part into a deck's net, notes), a def's call (its body may hold parts; its engine
  lines can carry `control:` themselves) and the settings.
- **A tempo on the control deck needs a time word.** `tempo 140` with no time word is the program's setting (in a section, the
  section's from its first bar): structure, and one spelling of it is enough. On the control deck a tempo is a change at a
  place (`at bar 16: control: tempo 128 -> 140 over 4 bars`), as it is with no head. The refusal is in `expand.rs`, not the
  parser, so the grammar stays one alternative.
- **The checks of the call are the line's with no head**, done where they were (the compiler); the control deck adds only its
  own refusals, in the parser, each with the line that would work.
- **One `tempo` production.** The grammar had two: `stempo` (a setting's, over `arith`) and `tempo` (a control's, over `value`,
  which may read a performed input). The parser reads both with one function, and neither takes an input
  (`at bar 4: tempo 120 + knob(1)` is refused), so `tempo` was wider than the language; the drift test found it once the control
  deck gave the sentence writer a second way to it (`control: tempo ... fader(B)`). Now `setting = "tempo" tempo` and `engine =
  ... | "tempo" tempo` share `tempo = arith [ "->" arith "over" dur [ shape ] ]`, the old `stempo` under the shorter name.
- **No CID moves.** Every program written before the control deck has the canonical text and CID it had
  (`crates/beatscript-api/tests/cid_pins.rs`, and `tests/control_deck.rs` against the same pins for the example programs djlive compiles
  alone).

## `macro`: the DJ macro catalogue

`macro` fires one entry of the DJ macro catalogue by name, for a length, on a deck: an engine call, so a line of the control deck
and, as `move` is, a line with no head.

```text
control: macro riser on A over 8 bars
at bar 24: control: macro cathedral on B over 2 bars
macro@1 breakdown over 4 bars on A
```

### Grammar

```ebnf
engine      = ... | MACRO NAME ( "on" deck [ span ] | span "on" deck ) ;
MACRO       = macro | macro@1 | ...      (* one word, the version glued with `@`: recognize.rs *)
```

`on DECK` and `over N bars` come in either order, each once, and print in the order `move` has (`macro riser on A over 8 bars`):
one order for the two booth calls. The canonical text writes the version only when it is not 1: `macro@1 riser over 8 bars on A`
is `macro riser on A over 8 bars`, one text and one CID.

### The catalogue, and `macro@1`

`djrust::fx::catalogue::ENTRIES` (djengine re-exports it) is the 39 effects the apps' Effects window shows, each in one category
and one engine family: 11 tricks (`riser tape_stop spin_back verb_freeze filter delay wash stutter pump half_time double_time`), 6
net macros (`cut double-drop beat-repeat tempo-anchor drop breakdown`), 13 master-chain macros (`filter-res build-crush
phaser-drone octave-up octave-down pitch-bend feel-chill feel-drive feel-ambient feel-euphoric feel-build feel-wind-down
feel-reset`) and 9 pan / shape / aux macros (`cathedral ping-pong auto-pan mono hard-left hard-right tighten loosen pulse`).

`macro@1` (`djlive::macros::V1`) is a copy of the catalogue's ids as of the engine this repo pins (djrust `7420204`), each with its
category, its family, whether it is a transition, whether it needs a net deck, and its length, compiled into djlive and never
edited: a stored program's `macro riser` means what it meant when it was stored. `tests/macros.rs` pins the table's CID and holds
it to the engine's catalogue entry by entry, so moving the pin to an engine whose catalogue changed fails until the new catalogue is
`macro@2`, a table of its own beside the first (`macros::VERSIONS`). A bare `macro` is `macro@1` for good, as `seed(techno, 42)` is
seeds v1 without saying so. The `macros` MCP tool lists a version's table.

| length | entries | `over N bars` | with no `over` | lowers to |
|---|---|---|---|---|
| bars | the 33 that are not transitions | runs N whole bars, 1 to 64 | 1 bar (the Effects window's own length); 2 for a sweep (`feel-build`, `feel-wind-down`, `feel-reset`) | `effects {action: fire, target, id, bars: N}` |
| sweep | `riser` | the noise rises over N bars | its own 4 | `effects {action: fire, target, id: riser, bars: N}` |
| held | `tape_stop`, `spin_back`, `verb_freeze` | held N bars, then let go | 1 bar | `effects {action: fire, target, id}`, then `effects {action: release, ...}` after N bars |
| own | `cut`, `double-drop` | refused | its own length | `effects {action: fire, target, id}` |

### What it lowers to: `effects fire`

`macro riser on A over 8 bars` is `effects {action: fire, target: A, id: riser, bars: 8}`: the registry verb `effects fire` (docs/CONTROL.md),
which is `djhost::fx_view::fire`, `Engine::engage(deck, id, Engagement::Bars(n))` with refusals of its own (an unknown id with the
nearest, `bars` outside 1 to 64, a length on a one-shot or none on the rest, a latch where the category has none, and the engine's
own: a net, pan or shape entry on a deck that plays no net). So the booth, the Effects window and the program fire the same thing.
The verb also takes `latch=true` (on until released), for the booth; a program has no latch (below).

**When it starts.** The engine starts every engagement on the deck's *next* bar line (at least a few milliseconds ahead), so a call
made on bar 24's line would land on bar 25. A macro's call is therefore made a beat ahead of a bar-line time: `at bar 24:` is sent at
bar 23 beat 3, `every 16 bars:` at 15 3/4, 31 3/4 ..., a section's first bar a beat before the section, and the macro starts on the
bar its line names. A time between bar lines (`at bar 3 beat 2`) is kept, and the macro starts on the next bar line, as a pad does.
Two places have no beat before them, and a macro there starts a bar later: the song's first bar (bar 0, and a line with no time
word in a program with no sections, offline), and `on next phrase`, which waits for its bar. The control's key is the line, so
`btw get` and the runtime's words name the line; its time is the call's.

**Offline.** The control net (`djlive::nets`) carries a macro as `fire A riser bars=8` (and a held one's `release A tape_stop`) on
the tick its call is made; the desktop's render (`djhost::beatscript_render::render`) and beatscript's stream (`render`, `listen`) apply it to
the offline engine through `fx_view::fire`. The same text renders the same samples every time, the same as the program without it
up to the bar its line names (`tests/macro_render.rs`, beatscript's `render_inproc.rs`). `listen`'s summary gains `macros` when a program
has one: each call's line and text, `at_bar` (the bar the engine starts it on, counted as the time words count), and `played`,
with the engine's reason in `refused` (and in `warnings`) when it did not play. `btw test` renders the arrangement only and lists a
macro line among what it leaves out.

**On beatscript.app.** The browser engine plays none of the catalogue: a macro line checks, and the page lists it as "not played
here: macro riser" (`engine-calls.js`), as it does the sugar nouns. Stream mode plays no control lines.

### What it refuses

| line | error | help |
|---|---|---|
| `macro rizer on A` | `rizer` is not in the macro catalogue of macro@1 | did you mean `riser`? (an id only a later version has: `` `vinyl-brake` is in macro@2 ``) |
| `macro@7 riser on A` | this BeatScript has macro@1, not `macro@7` | `macro riser ...` |
| `macro riser over 8 bars` | `macro riser` needs the deck it fires on | `macro riser on A` (the focused deck is the person's, not the text's) |
| `macro breakdown on A over 2 beats` | a macro takes whole bars | `... over 4 bars` |
| `macro breakdown on A over 65 bars` | a macro runs 1 to 64 bars | `... over 64 bars` |
| `macro cut on A over 2 bars` | `cut` is a one-shot: it runs its own length, so it takes no `over` | `macro cut on A` |
| `at bar 4 for 2 bars: macro breakdown on A` | a macro's length is its `over`, so it takes no `for` | `macro ID on A over 4 bars` |
| `deck A: macro riser on A` | `macro` takes no target: it names its deck itself | |
| `macro on A`, `macro riser on A on B` | the id is missing; the deck (or the length) twice | `macro riser on A over 8 bars` |

### Decisions

- **A transition with a length.** The design left open what a transition does with `over`. Decided by what each one's sound has:
  the riser's length *is* its sweep, so `macro riser on A over 8 bars` rises over 8 bars (the trick fired with `bars: 8` on the
  deck's next bar line; `engage` takes no length for a one-shot, so the verb fires the trick itself); a held one-shot (tape stop,
  spin back, reverb freeze) runs while its pad is held, so `over N bars` holds it N bars and lets it go (`effects release`), and
  with none it is held a bar, never left on (a program has no hand to lift); a song one-shot (cut, double drop) runs its own length
  and refuses `over`.
- **Past the panel's 4 bars.** 1 to 64 whole bars, the engine's own bound on `Engagement::Bars` (`engage` refuses 0 and above
  64). The window's 1 to 4 is a choice of its buttons, not of the engine; a build of 8 or 16 bars is ordinary in a program.
- **With no `over`**, the Effects window's own length: 1 bar, 2 for a sweep, the riser's 4, a held one held 1 bar, a song one-shot
  its own. The length is in the frozen table, so a later window or engine default does not move `macro@1`.
- **No latch in a program.** Nothing in a program lets a latch go (no pad lifts), so `macro` has a length, always; the verb's
  `latch` is the booth's.
- **The call goes a beat ahead** of a bar-line time so the macro starts on the bar its line names, live and offline alike (the
  runtime fires a time between bar lines when the position crosses it). The alternative, the engine's next bar after the line,
  would start every section's macro a bar late.
- **The verb keeps its name**, `effects fire`: the noun is the Effects window's, and the DJ language (run 1f12e380) builds on it as
  the macro surface.
- **`macro` is no reserved word**, like `control` and `drive`: it is the head only where a line's verb goes, before a word. A let, a
  def or a section may be called `macro`, and a scalar `let riser = 4` is not put in place of the id after it.
- **The table is data in djlive, not read from the engine at check time**, so the language stays pure and the meaning of a stored
  program cannot move with the engine; the agreement test is what ties them.

## Not done

- The browser engine does not play a macro line (beatscript.app lists it as not played here). djrust's wasm facade has
  `Dj.engage(deck, id, mode)`; a worklet op for it is the second proposal below.
- In the booth, djapp (djrust-vevr-se) has the registry's `effects fire` to answer: its Effects handler routes it to
  `djhost::fx_view::fire` once its copy of djhost has this commit.
- A riser given a length is fired as the trick itself, so the Effects window's pad does not light for it (`engage` records only
  what it fires); a length for a one-shot in `Engagement` (djrust) would let it.
- A macro at the song's first bar, or after `on next phrase`, starts a bar later (no beat before it to make the call in).
- The editors colour `control` as a keyword everywhere in VS Code and IntelliJ (one word list); djlive's highlighter and the
  page colour it as a head only before its colon.
- `btw fmt` offers no fix that moves an untargeted engine call onto the control deck (see the first proposal).
- The runtime's words and `btw get`'s `controls` name a line by its key, which has no head.

## Proposals (taste calls for Matt)

**Should the untargeted forms canonicalize onto the control deck later?** Today both spellings stand, and each is its own
canonical text. Making `control:` the only canonical spelling of an engine call would move the CID of every stored program that
has one (7 of the 41 example programs, and an unknown share of the saved ones), so it is not a printer change: it is a new
canonical form. The proposal: keep both for now; give `fmt` an opt-in fix (`btw fmt --control-deck`, and the editor's quick fix)
that rewrites a program onto the control deck as a new program with `forked-from` its old CID; and only if the control deck wins
in practice, make it the canonical form of a new format version (`#! btw 2`), the old texts reading and keeping their CIDs under
`#! btw 1`. Never silently. (The rename to BeatScript has since made the new format version, for its own reason: BeatScript 2,
`#! beatscript 2`, docs/BEATSCRIPT.md "Versions". It did not change the canonical form of control lines; a control-deck form
would now be a later version.)

**Should beatscript.app play macro lines?** djrust's wasm facade already has `Dj.engage(deck, id, mode)` (`momentary`, `latch`
or a number of bars), so a worklet op `{type: 'engage', deck, id, mode}` would play the catalogue in the browser: the bars entries
as they are, the riser through the `trick` live op with its `bars`, a held one-shot with `Dj.release` after its bars. It is a few
lines, but it cannot be heard from here (no browser in the build loop), so it is left for a pass with the page open.

**Should arrange be able to mutate the control deck per pass?** `arrange { }`'s mutations act on let patterns (`~ acid rot 2`),
and the control deck holds no pattern, so `rot`, `thin` or `up` mean nothing to it. Sections already sequence it (an engine line
lives in the section that wants it). The proposal: not now. If a pass-by-pass change is wanted, the narrow form is a switch, `@
control off` (the control deck's lines do not fire in the next section) and `~ control off` / `~ control on` (from here on), with
the counted forms (`~3 control off`); no mutation of a control's values.
