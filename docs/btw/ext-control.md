# The control deck (`control:`) and the design of `macro`

*Roadmap 6671 (the control deck, built) and 6670 item 2 (`macro`, designed here, not built). Code: `crates/djlive/src/parse.rs`
(`control_deck_body`, `control_head_next`, `control_head_at`), `ast.rs` (`Line::control`), `print.rs`, `expand.rs` (the head
dropped before lowering, the untimed tempo refused), `highlight.rs`; the grammar `crates/djlive/grammar.ebnf` (`headed`,
`engine`); tests `crates/djlive/tests/control_deck.rs`. The rule it follows is BTW.md's "Two halves: structure and engine".*

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
            | "queue" ( "play" | "pause" | "stop" | "next" | "loop" | "jump" INT | "add" WORD ) ;
```

`engine` is what `action` used to hold for the lines that take no target; it moved out of `action` into a production of its
own so the grammar can say what the control deck takes, and `control` (a line's body after an optional target) is `action |
engine | do | sing` as before, the same sentences. The grammar is exact about the control deck: the parser refuses nothing the
`engine | do | duck | mix` alternative writes (the drift test, `tests/grammar.rs`), and what it refuses is not in it. (The check
refuses one thing more, an untimed `control: tempo 140`: see Decisions.)

The head goes where a target goes: after the time word and its colon, before the call. `control: at bar 4: ...` is an error
(the time word comes first: `at bar 4: control: ...`), and so is a second head (`deck A: control: ...`, `control: master: ...`).

## What it takes, and what it lowers to

| on the control deck | is the line with no head | registry calls (BTW.md "Control") |
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
| `control: mvoe echo_out on B` | not an engine call the control deck takes | did you mean `move`? |
| `control: macro riser ...` | `macro` is designed and not built yet | `aux macro=cathedral deck=A bars=2`, `trick on deck=A kind=riser` |

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
  `mix`, which name their decks themselves and are engine calls (BTW.md's "Two halves" table puts them with the voice and the
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
  (`crates/btwapi/tests/cid_pins.rs`, and `tests/control_deck.rs` against the same pins for the example programs djlive compiles
  alone).

## The design of `macro` (not built)

`macro` would fire one entry of the DJ macro catalogue by name, for a length, on a deck: an engine call, so a line of the
control deck (and, as `move` is, a line with no head).

```text
control: macro riser on A over 8 bars
at bar 24: control: macro cathedral on B over 2 bars
macro@1 breakdown on A over 4 bars
```

**The catalogue.** `djrust::fx::catalogue::ENTRIES` (djengine re-exports it) is the 39 effects the apps' panels show, each in one
category and one engine family: 11 tricks (`riser tape_stop spin_back verb_freeze filter delay wash stutter pump half_time
double_time`), 6 net macros (`cut double-drop beat-repeat tempo-anchor drop breakdown`), 13 master-chain macros (`filter-res
build-crush phaser-drone octave-up octave-down pitch-bend feel-chill feel-drive feel-ambient feel-euphoric feel-build
feel-wind-down feel-reset`) and 9 pan / shape / aux macros (`cathedral ping-pong auto-pan mono hard-left hard-right tighten loosen
pulse`). The catalogue has rules of its own: a transition (`riser tape_stop spin_back verb_freeze cut double-drop`) is momentary
only, a sweep (`feel-build feel-wind-down feel-reset`) has bars but no latch, the rest run 1 to 4 bars or latch, and a net, pan or
shape entry needs a net deck. `Engine::engage(deck, id, Engagement)` fires any entry on the deck's next bar with those rules.

**Versioning.** `macro@1` is a copy of the catalogue's ids as of now (the 39 above, with each one's family and whether it is a
transition), compiled into djlive and never edited: a stored program's `macro riser` means what it meant when it was stored.
A later catalogue is `macro@2`, a table of its own beside the first. A bare `macro` is `macro@1`, for good, and the canonical text
writes the version only when it is not 1 (`macro riser`, `macro@2 riser`): every program written against `@1` keeps its text and
CID when `@2` arrives, as `seed(techno, 42)` is the generator of seeds v1 without saying so. An id that is not in the version's
table is an error with the nearest names (`macro rizer`: did you mean `riser`?); an id of `@2` written as `macro` says which version
has it.

**Syntax.** `macro ID [on DECK] [over N bars]`, both words in either order, printed in the order `move` has (`move bass_swap on
B over 8 bars`): one order for the two booth calls. `on DECK` is required in a program (the registry's default, the focused
deck, is the person's, not the text's), and `over N bars` takes whole bars as a move does; with none, the entry's own length.
`for N bars` is not needed (the length is the call's), and a transition with `over` is refused unless the engine learns to hold
one (below).

**What it would lower to, and what is missing in the engine.** No registry verb fires a catalogue entry by id for a length
today:

- `effects tap id=ID target=A` engages any entry, but for the Effects window's own length (`effects length`, state of the
  window), and a tap toggles (off when it is on): what it does depends on what was clicked before, not on the line.
- `aux macro=ID deck=A bars=N` (`Engine::fire_macro_traced`) fires a net, master or aux macro for N bars, but no trick; its registry
  schema lists only the 11 aux ids, so `do aux macro=breakdown` is refused by the checker though the handler takes it; and its
  `riser` is the master-chain riser (`MasterMacro::Riser`, merged out of the catalogue) where the catalogue's `riser` is the
  noise-riser trick: one id, two sounds.
- `trick on kind=K` fires the tricks, with `for N bars` to put one back.

A lowering by family in djlive (tricks to `trick on` / `off`, the rest to `aux macro=`) would copy the catalogue's rules into the
language and disagree with the panel on `riser`. The engine needs one verb instead: `effects fire id=ID target=DECK bars=N` (or
`latch`), which is `Engine::engage(deck, id, Engagement::Bars(n))` with its own refusals, so `macro riser on A over 8 bars`
lowers to `effects {action: fire, target: A, id: riser, bars: 8}` and the booth, the panel and the program fire the same thing.
Two engine questions come with it: what a transition does with a length (Matt's own example, `macro riser over 8 bars on A`, is
a transition: the noise riser sweeps over its `bars`, 4 by default, and `engage` passes it none), and whether a length past the
panel's 4 bars is allowed (`Engagement::Bars` takes any whole number; the panel offers 1 to 4). The browser engine plays none of
the catalogue: on beatscript.app a `macro` line would check and be listed as "not played here", as the sugar nouns are.

Until then, the catalogue is reachable through the registry: `control: aux macro=cathedral deck=A bars=2` (the 11 aux ids),
`control: trick on deck=A kind=riser bars=8`, `control: do deck master_macro deck=A id=build-crush bars=4`.

## Not done

- `macro` (above).
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
`#! btw 1`. Never silently.

**Should arrange be able to mutate the control deck per pass?** `arrange { }`'s mutations act on let patterns (`~ acid rot 2`),
and the control deck holds no pattern, so `rot`, `thin` or `up` mean nothing to it. Sections already sequence it (an engine line
lives in the section that wants it). The proposal: not now. If a pass-by-pass change is wanted, the narrow form is a switch, `@
control off` (the control deck's lines do not fire in the next section) and `~ control off` / `~ control on` (from here on), with
the counted forms (`~3 control off`); no mutation of a control's values.
