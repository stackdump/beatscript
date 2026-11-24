# BeatScript, the bitwrap language

*Beyond Swift.* **BeatScript** is the bitwrap language: one small language of our own (no embedded Lua, Rhai
or JS) that is a full front end to djapp. A **program** is a few lines of text, or a whole track. Pattern
lines become Petri nets on the decks; every other line is a call on the control-language registry, so
anything djapp does a program can do. You evaluate the text (Cmd+Enter, `btw eval`), and the change lands
on **the next bar**: no restart, no gap, no glitch. A program with an error is refused whole, and the last
good one keeps playing.

The language is called BeatScript, and `beatscript` is its name wherever a name is syntax: the command line `beatscript`,
files and snippets `.beatscript`, the MIME type `text/beatscript`, the editors' language id. It was called `btw` until
BeatScript 2 ("Versions"). The desktop booth (djrust) still spells its command noun `btw`: it names a registry noun,
not the language, and these are real commands: the registry noun `btw` (`live` and `livecode` still work), `do btw`,
`btw test`, `djctl btw eval`, the MCP tool `djapp_btw`, the window `btw`.

This page is the whole reference. The language grew in two steps: the base (mini-notation, control lines, `let` `def`
`repeat` `section` `arrange`, `evolve`, saved and shared programs, and sets: programs of programs), and fourteen extensions written in one run
(arithmetic, time finer than a bar, degrees and Roman numerals, Euclidean rhythms and polymeter, seeded chance,
`stack` and `then`, typed ports, tempo curves, shapes and lfos, `duck` and `mix`, `sing`, assertions, the performed
tier, links to the ontology). Each extension has one short section below ("The language extensions") and a long note,
`docs/beatscript/ext-<feature>.md`, with its grammar, semantics, decisions and what is not done. `docs/BEATSCRIPT-EXTENSIONS.md` is
the contract of that run, kept as history.

Code: `crates/djlive` (lexer, parser, mini-notation, expansion, IR, compile, diff, the hot-reload runtime, share links;
pure, no I/O; one module for each extension: `expr` `dur` `scale` `meter` `chance` `algebra` `ports` `mixmoves` `sing`
`tempo` `shape` `assertion` `perform` `linked`, and `set` for sets), `djhost::beatscript_env` (the registry, the engine's instruments, the voices,
the controllers' bank and the cached ontology, handed to the checker), `djhost::live_store` (saved `.beatscript` objects, their
JSON-LD and performances), `djhost::beatscript_render` (a song as an arrangement for the offline renderer),
`djhost::beatscript_test` (`btw test`), `djhost::beatscript_inputs` (the controllers behind `knob(3)` and its kin),
`djhost::ontology` (the vocabulary), `djapp/src/live_panel.rs` (the window, the booth's side) and
`djapp/src/control/live.rs` (the `btw` verbs).

```text
#! beatscript 2
tempo 126
key am

deck A: kick e(5,8) | snare ~ x ~ x | bass markov(am, 0.7) | hihat x*8 vel=60..90
deck B: harmony chord(am7) | arp arp(am7, up, 8)

every 16 bars: swap pairs with echo_out
on phrase: deck A: filter lp 12k -> 400 over 4 bars   # a build, every 8 bars
master: filter lp 2k -> 8k over 8 bars
```

Read it aloud: "deck A: kick, euclid five in eight; snare, rest hit rest hit; bass, markov in A minor,
density point seven. Every sixteen bars, swap pairs with an echo out. Master: filter low-pass two
kilohertz to eight kilohertz over eight bars."

## The rules in one screen

- A program is **lines**. A line is blank, a `# comment`, or one statement. A line that starts with
  `|` continues the statement above it.
- The first line is the version line, `#! beatscript 2`; a text without one is BeatScript 1 ("Versions").
- Words are lower case. Deck names are `A` and `B`. Numbers are plain: `5`, `0.7`, `2k` (= 2000),
  `50%` (= 0.5), and **wherever the language takes a number it takes arithmetic**: `tempo 120 + 6`,
  `at bar $i * 2 + 1`, `kick e($i % 4 + 1, 8)`.
- A statement is `[when:] [target:] body [when guard]`. Each head ends in a colon, and they read left to right:
  *when* (`every 16 bars`), *whose* (`deck A`, `master`), then *what*; a trailing `when $i % 2 == 0` keeps the line
  only where the condition holds.
- Time is **bars** (four beats), and a beat is a quarter of a bar: `over 2 beats`, `at bar 16 beat 3`. Seconds appear
  only where a sound or a check needs them (`attack=5ms`, `assert length == 3m30s`). A pattern is one **cycle** (one bar)
  long, and its terms share the cycle equally; `hihat/12` gives a part a cycle of its own.
- Text in, same net out: the same program text always gives the same net CIDs and the same program CID. Everything is
  **total** (no recursion, every count written out) and **deterministic** (chance is a hash of the text and the bar;
  nothing reads a clock, a random source or the machine).
- A mistake is an error with a line, a column, what was expected and the nearest name; the program is refused whole.

## Versions

A program starts with its version line:

```text
#! beatscript 2
tempo 126
deck A: kick x ~ x ~ | hihat x*8
```

**BeatScript 2** is the language that was called `btw`, renamed. Its meaning is the same line for line: every statement
of a BeatScript 2 program means what it meant before, and plays the same.

- **BeatScript 1 is the old spelling.** A text whose header block (the `#!` lines it starts with) says `#! btw 1`, or has
  no version line at all, is BeatScript 1. It reads, checks and plays as it always did, and keeps its old CID: every
  program, link and CID made before the rename means what it meant. `#! beatscript 3`, `#! btw 2`, `#! beatscript 1`, a
  bare `#! beatscript`, or two version lines are errors at the line.
- **The version line is part of the program.** The canonical text of a BeatScript 2 program keeps its version line first
  (`#! beatscript 2`, a newline, then the statements), and its CID is of `{"format":2,"text":<canonical text>,"type":"beatscript"}`.
  A BeatScript 1 program's CID is of `{"format":1,"text":<canonical text>,"type":"djrust.live"}`, its canonical text
  having no `#!` line ("Canonical form"). So the same music has a new CID in BeatScript 2. `check` says which version a
  text is in (`version`, 1 or 2) and the program's CID in the other one (`renamed_cid`).
- **It plays exactly as before.** The dice of chance and mutation and the control net are seeded from the program's
  BeatScript 1 CID, so a program and its upgrade play the same notes, generation for generation.
- **`fmt` upgrades.** The editor's fmt, the MCP `fmt` tool and `beatscript fmt` write the version line first and so turn a
  BeatScript 1 text into BeatScript 2 (the answer's `from_cid` is the CID of the text it was given);
  `beatscript fmt --keep-version` formats and keeps the version. Nothing else upgrades a text: what `eval`, `load`,
  `share`, `queue` and `push_private` format on the way in keeps its version, and its CID.
- **The store keeps both CIDs.** A stored BeatScript 1 program's BeatScript 2 twin is stored beside it, recorded as
  `isBasedOn` the old one with the derivation `renamed`. `/p/<old-cid>` redirects to `/p/<new-cid>`, and
  `/api/programs/<old-cid>` still answers its own text. Old share links still open: a BeatScript 1 link's text has no
  version line, so it verifies against its old CID.
- **Files and types.** `.beatscript` is the file extension, and a `.btw` file is still read (opened and imported). The MIME
  type is `text/beatscript` (`text/x-beatscript` and `text/btw` are still read). The JSON-LD context is `beatscript-context-v1.jsonld`,
  served at `https://beatscript.app/context/beatscript-v1.jsonld`; the old context stays served, unchanged, at
  `/context/v1.jsonld`.
- **The generators.** `gen@1` and `gen@2` print what they always printed, byte for byte (BeatScript 1, so their pins hold);
  `gen@3` is `gen@2`'s program written in BeatScript 2, its version line first; `gen@4` is `gen@3`'s with the acid genre's
  bass on the 303 (`inst=acid-bass`).

## Two halves: structure and engine

A program has two halves: its **structure** is the track's own source, and its **engine calls** ask the engine for what the text does not spell out.

- **Structure** is what you write and what the track is: the settings, the parts on `deck A:` and `deck B:`, `let`, `def`,
  `repeat`, `section`, `arrange` (and the mutations of `arrange { }`), doc comments, the phrase and form templates, and the
  sections and parts borrowed from a stored program by its CID. It is canonical and content-addressed: the canonical text is
  the program, the CID is the hash of that text, and anything brought in arrives as BeatScript text a person could have typed.
- **An engine call** reaches into the engine: a generator's track, a DJ move, a verb of the control registry, a ramp on a
  deck's or the master's strip. BeatScript treats the engine as a foreign-function interface. The text records the call,
  never its result, so a call that makes music is pinned and versioned: `seed(techno, 42)` is the generator of seeds v1, and
  always the same track (`seed@2(techno, 42)` names v2, in the text).
- **The two stay apart.** A part line holds notes and never a control call; a control line holds a control call and never a
  note. An engine call that makes music (a generator's track) feeds the parts: its result is written out as structure, a
  pattern or a section, before the program is lowered, so the browser, the phone and the booth only ever play plain BeatScript.

| structure: the track's own source | engine: calls into the engine |
|---|---|
| `tempo 126`, `key am`, `swing 0.1` | `seed(techno, 42).bass[17..24]`: the beats generator, seeds v1, written out as text before lowering |
| `deck A: kick x ~ x ~ \| bass c2 ~ c2 ~ inst=fm-bass`: parts, patterns, params | `master: filter lp 400 -> 8k over 8 bars`, `deck A: level 0.8`, `crossfader 0.5`: the mixer and the FX |
| `let`, `def`, `repeat`, `stack`, `then`, `evolve` | `move echo_out on B`, `swap pairs with filter_exit`: the booth's moves |
| `section drop 16 bars { }`, `arrange`, `arrange { }` with `~` and `@` | `do <noun> <verb> ...` and its sugar (`freeze`, `vocoder`, the wheel): the control registry |
| doc comments: the `#` lines above a `section`, `let`, `def` or `arrange` | `sing "words" on melody`, `duck`, `mix`: the voice and the mix |
| `arrange from template(rondo, @hands_up)`: a template is BeatScript with slots | `load`, `queue`, `play`, `stop`, `sync`, `mute`: the decks' transport |
| `section hook from <cid>.drop`, `<cid>.drop.bass`, `import`: another program's source, by its hash | `gen@2 techno seed=42 section=drop part=bass`: the seeded generator called with its own parameters, the result pinned by its CID ("Call the generator") |
| a form's roles, `template(rondo, @hands_up, episodes=template(passacaglia, ground=ode_to_joy))`: themes, notes, other programs' parts and sections, other forms, written in as text (a seed in a role stays an engine call) | *coming:* `macro riser over 8 bars on A`, the DJ macro catalogue by name (designed: docs/beatscript/ext-control.md) |
| `use theme ode_to_joy as tune`, `use <cid>.drop as hook`, `use template(passacaglia).ground as ground`: a name for a source, on a line of its own, written out where the name is used ("Name a source: `use`") | `use gen@2 house seed=7 section=drop part=bass as walk`: a name for an engine call, still pinned and resolved as one |
| | `control: move echo_out on B`, `at bar 12: control: swap pairs with filter_exit`: the control deck, beside `deck A:`, `deck B:` and `master:`, holds only engine calls, sequenced by sections like any deck ("The control deck") |

One program that uses both:

```text
#! beatscript 2
tempo 126
key am

let groove = x ~ x ~ x ~ x [x x]                          # structure: a let
section build 8 bars {                                     # structure: a section
  deck A: kick groove | bass seed(techno, 42).bass[17..24] inst=fm-bass   # parts; seed(): the generator, pinned
  deck B: arp arp(am7, up, 8)
  master: filter lp 400 -> 8k over 8 bars                 # engine: the master's filter, a ramp
}
section drop 8 bars {
  deck A: kick x x x x | bass seed(techno, 42).bass[17..24] inst=fm-bass
  deck B: arp arp(am7, up, 8)
}
on phrase: move echo_out on B                              # engine: a move of the booth
arrange build drop
```

### What a reference resolves to

A program's CID is of its text as written: the recipe, with its references in it. `check` (and `listen`, and the MCP tools
`generate` and `genre_template`) also says what the recipe makes:

- refs lists each reference of the program (a gen@N call, a seed(...), a template(...), a <cid>.section borrowed from a stored
  program): its kind, half (engine or structure), call, place (part, let, section or arrange), name, line and col, resolved
  (the CID it is pinned to), generator, fitted (what a gen call took from the program: key, bpm) and track. With `expand`
  (`beatscript check --expand`, `"expand": true` on `/api/check` and the MCP check) each one also carries its `expansion`. A `use`
  line is a reference of kind `use`: its call is the source, its name the name it gives, its place what the name is (section,
  let, or use for a source of a role or a phrase).
- imports lists each `use SOURCE as NAME` line of the program: its name, what the name is (is: a section, a line, a source for
  a role or a phrase), from (the source, in canonical spelling), source (its kind: stored, gen, template, form, theme, phrase or
  genre), resolved (what it is pinned to: a CID, or the library's CID for a theme or a phrase), line and col, and what (a
  sentence). `check`, `expand` and `listen` answer it.
- expanded_cid is the CID of the program with every reference written out as plain BeatScript: cid addresses the recipe,
  expanded_cid the result; the same recipe always gives the same expanded_cid, and a program with no references has
  expanded_cid equal to cid.
- resolution is the manifest: each reference once, with the CID it is pinned to and the generator and version that made it
  (seeded@2), and for a gen call the key and bpm it was generated in. A reference written out inside another one's expansion
  (a template's) is listed with `nested: true`, and its own pin: a gen call a role wrote is resolved in the key and tempo of the
  written-out program. And a template's entry in resolution has parts: each role its reference binds, at any depth (a role of a
  form bound to a role is a part of that part), as {path, role, kind, call, cid, resolves, generator, parts}: path is where it
  stands (episodes/ground), kind what it was bound to (gen, stored, theme, notes or template), call the source as bound, cid the
  CID of what it is written out as (a program of its own: the key and tempo it plays in and the lines its binding wrote, every
  reference in them written out), resolves the CID its source is pinned to; the same recipe always gives the same parts and part
  CIDs. Asked to expand, each part also carries its text (docs/beatscript/ext-template.md, "Nested parts").

`listen` marks each part a reference made with `from` (its call, kind, resolved CID, place and the sections it made it in; a
`use` line's also its `name`), and a gen call that was fitted to the program's key or tempo says so in its `notes`, in check's
words.

A program with references that is stored on beatscript.app is stored with its **expansion** beside it: the program written out
(its `expanded_cid`), canonical, as a program of its own under the same storage gate, with the recipe's licence, credits and name.
`/p/<expanded_cid>` loads it and it plays as the recipe does. The two name each other (the recipe's meta `expands_to`, written once
the expansion is stored; the expansion's `expanded_from`, and its `based_on`: each reference the recipe wrote out into it, with its
pin, never the expansion itself), and so do their JSON-LD (`expandsTo`; `expandedFrom` and `isBasedOn`) and lineage. First write
wins: an expansion someone stored first is the one, with the rights they gave it. An expansion is a copy, so it is stored only
when every program it copies is CC0 or the recipe author's own under the recipe's licence, and when it is within a program's size
and the store's room; else the answer says why. The answers say `expanded_cid`, and `expanded_url` and `expansion_license`, or
`expansion_note` (docs/beatscript/ext-template.md, "The stored expansion").

## Grammar (EBNF)

The grammar is [`crates/djlive/grammar.ebnf`](https://beatscript.app/grammar.ebnf), the language's one source of
truth. The block below is generated from it (do not edit it here), and so is the [Grammar page](https://beatscript.app/grammar),
which draws every production as a railroad diagram. `crates/djlive/tests/grammar.rs` holds it to the parser both ways:
every example program is a sentence of it, and every sentence it writes is a program the parser reads.

`{ x }` zero or more, `[ x ]` optional, `( a | b )` choice, `"x"` a literal word or mark, `(* ... *)` a comment; an
UPPER-CASE name that is no production is a kind of token (below). Whitespace separates tokens and is otherwise free; a
line end ends a statement (except before `|`).

<!-- grammar.ebnf: generated from crates/djlive/grammar.ebnf; BEATSCRIPT_BLESS=1 cargo test -p djlive --test grammar rewrites it -->
```ebnf
(* == Programs and lines == *)
program     = { line } ;
line        = [ statement ] NEWLINE ;                         (* a comment is "#" to the end of the line; it is not a token *)
statement   = setting | headed | let | def | repeat | section | arrange | arrange_block | start | launch | import | end | loop
            | evolve | wire | assert | sample | use | set | station ;

(* == Settings == *)
setting     = "tempo" tempo
            | "swing" arith
            | "key" KEY ;                                     (* a setting reads no input: it is the program's own *)
tempo       = arith [ "->" arith "over" dur [ shape ] ] ;     (* tempo 126   tempo 100 -> 130 over 8 bars exp *)

(* == Lines: when, where, what == *)
headed      = "deck" deck ":" parts [ guard ]                (* a deck's parts: no time word before them *)
            | [ timing ":" ] [ target ":" ] control [ cguard ]
            | [ timing ":" ] "control" ":" ( engine | do | duck | mix ) [ cguard ]   (* the control deck *)
            | [ timing ":" ] ( invoke | duck | mix ) [ cguard ] ;   (* a def's call, a duck and a mix name their decks themselves *)
timing      = when [ "for" dur ] | "for" dur ;
when        = "every" dur [ "from" ( arith | pos ) ]          (* every 16 bars   every 2 beats   every 4 bars from 8 *)
            | "on" "phrase"                                   (* every 8 bars: a phrase *)
            | "on" "next" "bar"
            | "on" "next" "phrase" [ "8" | "16" | "32" ]
            | "at" pos                                        (* once, on that place of the program *)
            | "after" NAME                                    (* once, where that section ends *)
            | "before" NAME dur ;                             (* once, that long ahead of that section *)
dur         = arith ( "bar" | "bars" | "beat" | "beats" ) ;   (* 4 bars   1/2 bar   2 beats *)
pos         = "bar" arith [ "beat" arith ] ;                  (* bar 16   bar 16 beat 3   a beat is 0 to 3 *)
guard       = "when" expr ;                                   (* kept where it holds *)
cguard      = "when" cexpr ;                                  (* a control line's guard may read an input *)
target      = "deck" deck | "master" ;
deck        = "A" | "B" | PARAM ;                              (* PARAM: in a def body, filled in when it is called *)
control     = action | engine | do | sing ;

(* == Parts == *)
parts       = pitem { "|" pitem } ;                           (* only after "deck X:" and with no time word *)
pitem       = part                                            (* each item between `|` has a part in it *)
            | "stack" "(" pargs ")"                           (* over parts: `stack` is `|` *)
            | "then" "(" pargs ")" ;                          (* over parts: the per-bar alternation <p q> *)
pargs       = { pexpr "," } pitem { "," pexpr } ;
pexpr       = pitem | "silence" ;                             (* silence: the unit of both *)
part        = role [ "/" INT ] pattern { param }              (* hihat/12 x x ~ x : a cycle of 12 sixteenths *)
            | role ref { param } ;                            (* bass seed(techno, 42)[17..24]: a generated part's bars; bass <cid>.drop: a stored program's *)
role        = "kick" | "snare" | "hihat" | "clap"             (* drum parts: a hit plays the drum's note *)
            | "bass" | "melody" | "harmony" | "arp" ;         (* tonal parts: a hit plays the key's root *)
param       = pname "=" pvalue                                (* vel=70..100  oct=2  len=3  inst=sub-bass  glide=60 *)
            | "voice-lead" [ "=" ( "on" | "off" ) ] ;         (* a switch, on when it is bare *)
pname       = "vel" | "oct" | "len" | "inst" | "seed" | "kit" | "sound" | "route"   (* velocity, octave, length, voice, its generators' seed, a drum kit, a sound, where it plays *)
            | "glide" | "decay" | "pitch-drop"                (* tonal, an 808: glide= decay= ms, pitch-drop= 1 to 24 semitones *)
            | "cutoff" | "res" | "env" | "accent" | "drive" ; (* the acid voice's knobs: cutoff= Hz 20 to 12000, the rest 0 to 100; not reserved words *)
pvalue      = arith [ ".." arith ] | WORD | PARAM ;           (* a number, a range drawn per hit, a name; a $name in a body *)

(* == Mini-notation: the rhythm of a part == *)
pattern     = term { term } ;
term        = atom [ mark ] [ "*" ( INT | "(" arith ")" ) ] { "?" [ prob ] } [ "~" INT ] ;   (* x*4  c2!  e2^  x:3  x*8?0.7 *)
                                                              (* sugar: x? is x?0.5, c2~3 is c2 ~ ~ *)
mark        = ( "!" | GHOST ) [ "^" ] [ "&" ] [ RATCHET ] | "^" [ "!" | GHOST ] [ "&" ] [ RATCHET ]
            | "&" [ RATCHET ] | RATCHET ;                     (* accent ghost slide flam ratchet, any order *)
atom        = "x"                                             (* a hit *)
            | "~"                                             (* a rest *)
            | NOTE                                            (* c3  f#2  bb4 *)
            | INT                                             (* a scale degree of the key: 1 3 5 *)
            | "(" arith ")"                                   (* a degree computed: ($i + 1) *)
            | degrees                                         (* deg(1 b3 5' ~) *)
            | ROMAN [ "/" INT ]                               (* i  VI  V7/7: a chord of the key; /3 /5 /7 an inversion *)
            | PARAM                                           (* a def's parameter: $chords *)
            | "[" pattern { "," pattern } "]"                 (* a group: squeezed into one slot; commas stack *)
            | "<" pattern ">"                                 (* alternation: one term per cycle *)
            | choose                                          (* choose [a b c] weights 3 1 1 *)
            | call
            | NAME ;                                          (* a let pattern's name *)
call        = NAME "(" [ arg { "," arg } ] ")" ;              (* e(5,8)  markov(0.6, shift=2)  arp(am7, up)  chord(am)  maybe(p, 0.5) *)
arg         = arith | KEY | CHORD | ROMAN [ "/" INT ] | NAME | NAME "=" ( arith | WORD ) | "[" pattern "]" | degrees | call | PARAM ;
                                                              (* a chord of chord() and arp(): a symbol (am7) or a numeral (V7  bVII  V7/7), the numeral read in the key in force *)

(* == Degrees, Roman numerals, chance == *)
degrees     = "deg" "(" dstack ")" ;                          (* one slot, its terms squeezed into it like [..] *)
dstack      = dseq { "," dseq } ;
dseq        = dterm { dterm } ;
dterm       = dleaf [ mark ] [ "*" INT ] | "~" | "x" | "[" dstack "]" | "<" dseq ">" | "(" ( dterm | arith ) ")" ;
dleaf       = ( INT | SDEG ) { "'" } { "," } ;                (* 5' an octave up, 1, down; b3 flat, #4 sharp *)
prob        = NUMBER | "(" arith ")" ;                        (* 0 to 1, or a percent: ?0.3  ?30%  ?(1 / 4) *)
choose      = "choose" "[" pattern "]" "weights" weight { weight } [ "seed=" INT ] ;   (* one weight for each option *)
weight      = NUMBER | "(" arith ")" ;

(* == Control: what the mix does == *)
action      = "filter" ( "lp" | "hp" ) change                 (* hertz: 20 .. 20k *)
            | "filter" ( "off" | "open" )
            | ( "delay" | "reverb" | "crush" | "drive" | "resonance" ) change   (* 0 .. 1: how much *)
            | ( "level" | "low" | "mid" | "high" ) change     (* deck X: / master: level *)
            | "load" SPEC                                     (* deck X: *)
            | "play" | "stop" | "sync" | "mute" | "unmute" ;
engine      = "crossfader" change                             (* no target, or control: *)
            | "tempo" tempo                                   (* at bar 16: tempo 128 -> 140 over 4 bars *)
            | "swap" "pairs" [ "with" MOVE ]                  (* the booth's: the desktop app plays these *)
            | "move" MOVE "on" deck [ span ]
            | "load" "pair" SPEC [ "group" INT ]
            | "queue" ( "play" | "pause" | "stop" | "next" | "loop" | "jump" INT | "add" WORD )
            | MACRO NAME ( "on" deck [ span ] | span "on" deck ) ;   (* macro riser on A over 8 bars: the DJ macro catalogue, by id; MACRO is macro or macro@1 *)
change      = value                                           (* a plain value *)
            | value "->" value span [ shape ]                 (* a ramp: 2k -> 8k over 8 bars exp *)
            | lfo ;                                           (* a value that keeps moving *)
span        = "over" dur ;
value       = carith ;                                        (* a control's value may read inputs: 200 + knob(3) * 7800 *)
MOVE        = "filter_exit" | "echo_out" | "loop_roll" | "bass_swap" | "phrase_sync" | "vocoder_handoff" ;
SPEC        = NAME ":" INT | NAME ;                           (* dnb:2602  acid-bass *)
do          = ( "do" WORD | SUGAR ) [ WORD ] { STRING } { NAME "=" doval } [ span [ shape ] ] ;   (* the control registry: noun, verb; SUGAR: a noun said with no `do` (vocoder, freeze, wheel ...) *)
doval       = arith [ "->" arith ] | WORD | STRING | "[" [ doval { "," doval } ] "]" | lfo ;

(* == Shapes and lfo == *)
shape       = "linear" | "exp" | "log" | "s-curve" | "step" INT ;
lfo         = "lfo" wave dur { "depth=" value | "center=" value | "phase=" NUMBER } [ span ] ;   (* lfo sine 2 bars center=3k depth=1k *)
wave        = "sine" | "tri" | "square" | "saw" ;

(* == Arithmetic and guards == *)
arith       = sum ;                                           (* worked out once, when the program is read, over exact rationals *)
sum         = product { ( "+" | "-" ) product } ;
product     = unary { ( "*" | "/" | "%" ) unary } ;           (* "/" is exact; "%" is Euclidean *)
unary       = [ "-" ] primary ;
primary     = NUMBER | PARAM | "(" arith ")" | ( "min" | "max" ) "(" arith { "," arith } ")" ;
carith      = csum ;                                          (* arithmetic that may read inputs: only in a control *)
csum        = cproduct { ( "+" | "-" ) cproduct } ;
cproduct    = cunary { ( "*" | "/" | "%" ) cunary } ;
cunary      = [ "-" ] cprimary ;
cprimary    = NUMBER | PARAM | "(" carith ")" | ( "min" | "max" ) "(" carith { "," carith } ")" | input ;
expr        = or ;                                            (* a guard: kept when it is true *)
or          = and { "or" and } ;
and         = not { "and" not } ;
not         = [ "not" ] compare ;
compare     = operand [ cmp operand ] ;                       (* words compare with == and != only *)
operand     = sum | WORD ;
cexpr       = cor ;                                           (* a control line's guard *)
cor         = cand { "or" cand } ;
cand        = cnot { "and" cnot } ;
cnot        = [ "not" ] ccompare ;
ccompare    = coperand [ cmp coperand ] ;
coperand    = csum | WORD ;
cmp         = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
input       = "knob" "(" INT ")" | "pad" "(" INT ")" | "fader" "(" deck ")"   (* performed: only in a control value or guard *)
            | "held" "(" "key" ")" | "held" "(" PITCH ")" ;    (* PITCH: a note with its octave, c4 *)

(* == Structure: lets, defs, sections, the arrangement == *)
let         = "let" NAME "=" ( sexpr | arith | ref | TOKEN { TOKEN } ) ;   (* one token is a value; more tokens are a pattern *)
def         = "def" NAME "(" [ NAME { "," NAME } ] ")"
              [ "in" ":" type { "," type } ] [ "out" ":" type { "," type } ]
              "{" { def_line } "}" ;                          (* $NAME inside, filled in when it is called *)
def_line    = [ statement | emit ] NEWLINE ;                  (* a def's body line; `emit` says what an out port carries *)
invoke      = NAME "(" [ tokens { "," tokens } ] ")" ;        (* a def's call *)
tokens      = TOKEN { TOKEN } ;
repeat      = "repeat" INT [ "every" INT "bars" ] "{" { body_line } "}" ;   (* the count is written out, 1 to 64 *)
body_line   = [ statement ] NEWLINE ;                         (* a repeat's body line: $i is the pass, 0 up *)
section     = "section" NAME INT "bars" "{" { line } "}"     (* 1 to 256 bars; a `key` may open the body *)
            | "section" NAME "from" ref ;                     (* section intro from seed(techno, 42)[1..8]   section hook from <cid>.drop *)
arrange     = "arrange" sexpr { sexpr }
            | "arrange" NAME ":" sexpr { sexpr } [ arr_end ]   (* with a NAME it is one of several arrangements *)
            | "arrange" "from" ( seed | template )            (* the whole of a generated track: arrange from seed(techno, 42), or of a phrase: arrange from template(deep-house, @tempest_such_stuff) *)
            | "arrange" "from" gen ;                          (* the whole program a generator call prints: arrange from gen@2 techno seed=42 *)
arr_end     = "then" NAME | "loop" [ INT ] | "end" ;           (* only after a NAME: see "Several arrangements" *)
start       = "start" NAME ;                                  (* the named arrangement that plays first; default the first written *)
launch      = "launch" NAME "on" "pad" "(" INT ")" [ "next" ( "bar" | "phrase" [ "8" | "16" | "32" ] ) ] ;   (* a hit of the pad brings the scene, a section or an arrangement, in on that bar line: 8 bars by default *)
sexpr       = NAME                                            (* a section, or a let that names a composition *)
            | "silence"                                       (* the empty section *)
            | "stack" "(" sexpr { "," sexpr } ")"             (* at the same time *)
            | "then" "(" sexpr { "," sexpr } ")" ;            (* one after another *)
end         = "end" [ "at" "bar" INT | "after" NAME ] ;       (* a song ends after its last section by default *)
loop        = "loop" ;                                        (* a song starts again at its end *)

(* == Arrange mutations == *)
arrange_block = "arrange" "{" { arr_gap } NAME { arr_gap | NAME } "}" [ block_end ]   (* at least one section; see "Arrange mutations" *)
            | "arrange" NAME ":" "{" { arr_gap } NAME { arr_gap | NAME } "}" [ arr_end ] ;   (* a named arrangement with mutations *)
arr_gap     = NEWLINE | "," | mutation NEWLINE ;               (* a mutation runs to the end of its line *)
mutation    = ( "~" | "@" ) [ INT ] ( switch | mut_target mut_op { "|" mut_op } { "," mut_target mut_op { "|" mut_op } } ) ;   (* ~ from here on, @ the next section only; ~3: every 3rd pass *)
switch      = "control" [ NAME ] ( "on" | "off" ) ;           (* the control deck's lines fire or not: all of them, or one call's (~ control freeze off); a line of its own *)
mut_target  = GLOB | NAME ;                                   (* a let pattern, or a glob: acid_* *)
mut_op      = NAME { NUMBER | "-" NUMBER | NAME } ;           (* rot 2  thin 0.1  oct -1  morph b 0.5  jux rev: the operators of "Arrange mutations" *)
block_end   = "end" | "loop" | "loop" INT [ "then" { NAME } "end" ] ;   (* default end; loop N: 1 to 256 passes, then a coda *)

(* == Start from a seed == *)
ref         = seed | gen | stored ;                           (* music from elsewhere, written out as BeatScript before the program is read *)
seed        = SEED "(" NAME "," INT [ "," structure ] ")" [ "." role ] [ "[" INT [ ".." INT ] "]" ] ;   (* a track of the beats generator: genre, seed; a part, bars from 1; SEED is seed (seeds v1) or seed@2 (v2: with its genre's swing) *)
stored      = CID [ "." NAME ] [ "." role ] ;                 (* a saved program's section or part, by its CID: <cid>.drop  <cid>.drop.bass; a bare <cid> is a program with no sections [host] *)
gen         = GEN NAME "seed" "=" ( INT | NAME ) { gen_param } ;   (* gen@2 techno seed=42 section=drop part=bass: the seeded generator, called; GEN is gen@2 or a bare gen [host] *)
gen_param   = "bpm" "=" NUMBER | "key" "=" KEY | "length" "=" ( "short" | "standard" | "long" ) | "section" "=" NAME | "part" "=" role ;
structure   = "standard" | "loop" | "minimal" | "extended" ;  (* the generator's song structures; standard is the default *)

(* == Phrase templates == *)
template    = "template" "(" NAME "," ( PHRASE | STRING | NAME ) { "," binding } ")" ;   (* a template's name and a phrase, a phrasebook id or words in quotes (NAME: one a `use phrase` line named): the phrase is the vocal line of a track; then a form's roles *)
binding     = NAME [ "." role ] "=" ( source | [ "-" ] INT ) ;   (* a role of the form and what fills it: ground=gen@1 dubstep seed=54876 part=bass; one part of a seed role: a.bass=[d2 ~ a1 ~]; a form's number: delay=6 interval=-12 *)
source      = gen | NAME | notes | form | stored | track ;    (* an engine call; a public-domain theme by id (ode_to_joy); notes; another template, or one section of it; a stored program's part or sections; a stored set's track *)
notes       = "[" pattern "]" | "<" "[" pattern "]" { "[" pattern "]" } ">" ;   (* a bar, or bars one after another *)
form        = "template" "(" NAME { "," binding } ")" [ "." NAME [ "." role ] ] ;   (* a form inside another takes no phrase: forms nest 3 deep; .both is one of its sections, .both.arp that section's part *)
track       = CID "." NAME [ "." NAME ] [ "." role ] ;        (* a stored set's track by number: <cid>.track2, its section <cid>.track2.drop, its part <cid>.track2.drop.bass [host] *)

(* == Evolving == *)
evolve      = "evolve" "every" INT "bars" "{" { NEWLINE } { rule NEWLINE { NEWLINE } } [ bound NEWLINE { NEWLINE } ] "}" ;   (* a rule a line; the bound last *)
rule        = ( role | "deck" deck ) ":" op { "|" op } [ "when" "gen" [ "%" INT ] cmp INT ] ;
op          = ( "rotate" | "transpose" | "octave" | "invert" | "reverse" | "thin" | "fill" | "slow" | "fast" ) { NUMBER | "<" NUMBER { NUMBER } ">" } [ "seed=" INT ] ;   (* seed= on thin and fill *)
bound       = "stop" "when" "gen" ( "==" | ">=" | ">" ) INT | "for" INT "gens" ;

(* == Typed ports == *)
type        = "chords" | "key" | "bass" | "melody" | "drums" | "rhythm" | "bars" | "tempo" | "pattern" ;
emit        = "emit" [ type | "out" ] tokens ;                (* a line of a def body: what an out port carries *)
wire        = "wire" NAME "." port "->" NAME "." port ;       (* at the top: wire changes.out -> walker.chords *)
port        = type | "in" | "out" ;

(* == Mix moves and sung lyrics == *)
duck        = "duck" deck "under" deck "." role db { "attack=" time | "release=" time } ;
mix         = "mix" "{" { NEWLINE } mixitem { ( "," | NEWLINE ) { NEWLINE } mixitem } { NEWLINE } "}" ;   (* in a section: put back where the section ends *)
mixitem     = deck "level" arith | deck "eq" band arith [ "db" ] | "master" "level" arith | deck "pan" arith ;
band        = "low" | "mid" | "high" ;
db          = DECIMAL "db" ;                                  (* glued: 6db *)
time        = DECIMAL ( "ms" | "s" ) ;                        (* glued: 5ms *)
sing        = "sing" ( lyric | PHRASE | NAME ) "on" role { "bars=" INT | "key=" KEY | "voice=" NAME } ;   (* a tonal role; PHRASE: a phrasebook id, @hands_up; NAME: a phrase a `use phrase` line named *)
lyric       = STRING ;   (* its text is lyric_text; docs/beatscript/phonemes.md *)
lyric_text  = { TEXT | "/" SPACE | escape } [ "/" { TEXT } ] ;   (* "/" and a space is a phrase break; "/" and a phoneme opens an escape, to the next "/" (with none, a break too); a word touching "=" before an escape is its label, kept and not sung: jingle=/j ii ng g uh l/ *)
escape      = "/" PHONES "/" ;   (* PHONES (beatscript-v1): aa ae uh aw eh er ii ee uu oo ay ie oy ow oh p b t d k g f v s z sh zh th dh h m n ng l r w y ch j, "_" a rest, "-" cuts a word, ":" holds a vowel one more note, "!" accents it *)

(* == Assertions == *)
assert      = "assert" ( "ends"
                       | "period" [ "(" NAME ")" ] cmp INT
                       | "length" ( ( "==" | "!=" ) clock [ "±" clock ] | ( "<" | "<=" | ">" | ">=" ) clock )
                       | "level" leveldeck cmp levelexpr [ "in" NAME ]
                       | "key" ( "==" | "!=" ) KEY [ "in" NAME ]
                       | "never" subject ( "beside" | "during" ) subject [ "in" NAME ]   (* assert never A.snare beside A.snare   assert never B during sing: refused where it is false *)
                       | subject "ends" "on" INT [ "in" NAME ] ) ;                        (* assert bass ends on 1: the lowest note of its last onset is that degree of the key *)
subject     = "sing" | ldeck | [ ldeck "." ] role ;                                  (* sing: the vocal; a deck; A.snare; bass on both decks *)
clock       = DECIMAL "m" [ DECIMAL "s" ] | DECIMAL "s" | DECIMAL "ms" ;   (* 3m30s  90s  500ms *)
leveldeck   = { lfactor lop } ldeck { lop lfactor } ;         (* a level read: it names a deck *)
lop         = "+" | "-" | "*" | "/" ;
ldeck       = "A" | "B" | "master" ;
levelexpr   = lterm { ( "+" | "-" ) lterm } ;                 (* arithmetic over the words A, B and master: their levels *)
lterm       = lfactor { ( "*" | "/" ) lfactor } ;
lfactor     = NUMBER | ldeck | "(" levelexpr ")" ;

(* == Sets, samples, imports == *)
set         = "set" "{" { NEWLINE } { ( set_play | set_mix | set_next ) NEWLINE { NEWLINE } } "}" ;   (* programs of programs: an item a line *)
set_play    = "play" ( NAME | STRING | gen ) [ "until" until_when ] ;   (* a saved program, its CID in the canonical text, or a generator call [host] *)
set_next    = "next" step [ "until" until_when ] ;            (* the track before it, a generator call, with its seed stepped [host] *)
until_when  = TIME | "bar" INT | NAME | "after" NAME ;        (* a time, a bar of the track, a section's start or end *)
set_mix     = "mix" INT ( "bar" | "bars" ) "with" TRANSITION [ "," "tempo" NUMBER "->" NUMBER ] ;
TIME        = DECIMAL UNIT { DECIMAL UNIT } ;                 (* units glued: 20m  90s  1h  3m30s *)
TRANSITION  = "crossfade" | "filter_exit" | "echo_out" | "cut" ;

(* == Radio: stations == *)
station     = "station" "{" { NEWLINE } { station_line NEWLINE { NEWLINE } } "}" ;   (* endless and deterministic: position N is always the same track [host] *)
station_line = "from" gen | "next" step | "forms" NAME { NAME } ;   (* the first track's call; the step from a track to the next; the forms in turn *)
step        = "seed" "=" ( "+" INT | "~" ) ;                 (* seed=+1 counts on; seed=~ draws from a hash of the track before's CID *)
import      = "import" ( NAME | STRING ) ;                    (* a snippet's name in quotes, or its CID [host] *)
sample      = "sample" NAME "from" STRING [ "license" license ] [ "else" NAME ]
            | "loop" STRING "as" NAME [ "else" NAME ] ;       (* restricted sounds: a local file, an Apple Loop by its hash *)
license     = "cc0" | "cc-by" | "all-rights-reserved" | "restricted-private" ;
use         = "use" "restricted"                                (* the program is private: it may use restricted assets *)
            | "use" use_source "as" NAME ;                        (* a name for a source, written out where it is used (docs/beatscript/ext-roles.md) *)
use_source  = stored                                            (* <cid>.drop: a section; <cid>.drop.bass: a line; <cid>: a source for a role *)
            | gen                                               (* gen@2 techno seed=42 section=drop, part=bass: a section or a line; with neither, a source for a role *)
            | "template" "(" NAME { "," binding } ")" [ "." NAME ]   (* template(passacaglia, ground=...): a form, for a role; .ground: a canned section of it *)
            | template "." NAME                                 (* template(deep-house, @tempest_such_stuff).night: a canned section, sung with that phrase *)
            | "theme" NAME                                      (* a theme of the public-domain library: a line, in the program's key *)
            | "phrase" PHRASE                                   (* a phrasebook line: a phrase, for sing and template *)
            | GENRE NAME "." NAME ;                             (* genre@1 techno.drop: a section of a genre template, frozen *)
```
<!-- /grammar.ebnf -->

`NAME` is a lower-case word (letters, digits, `_`, and `-` between letters), `WORD` any word. `INT` is a whole number
(arithmetic that comes to one is fine: `repeat 2 + 1`); `NUMBER` also takes a fraction and the `k` / `%` marks, and
`DECIMAL` is a number with no mark, before a glued unit (`UNIT`: `6db`, `5ms`, `2s`, `3m30s`, `1h30m`; `hz` after a
filter's cutoff, `filter lp 300hz`, is the number and nothing else; the hours of `1h` are read only after a set's
`until`). `NOTE`, `PITCH` (a note with its octave), `KEY`, `CHORD` and `ROMAN` are pitch words read as one token
(`f#2`, `c4`, `d:dorian`, `am7`, `ii°`); `SDEG` a written degree in `deg( )` (`b3`, `#4`); `PARAM` a def's or a
repeat's `$name`; `SUGAR` a control noun said without `do`; `STRING` a double-quoted string; `TOKEN` any token; `GLOB` a
name with `*` in it; `CID` a saved program's id; `SEED` `seed` or `seed@2`; `GHOST` a ghost's `.` and
`RATCHET` a ratchet's `:N` (`x.` `x:3`). `±` may be written `+-` after
a unit (`3m30s +- 5s`). In a lyric, `PHONES` is an escape's text:
`aa ae uh aw eh er ii ee uu oo ay ie oy ow oh p b t d k g f v s z sh zh th dh h m n ng l r w y ch j`, `-` cuts a word, `_` rests,
`:` holds a vowel, `!` accents it; `/` and a space: a phrase break. A phrase's `NAME` is one `use phrase @id as NAME` gave;
`use theme ode_to_joy as tune` names a theme ("Name a source").

Some marks must **touch** the token before them, because a space would change what they mean: `hihat/12`, `x?0.3`,
`A.kick`, `6db` `120ms` `3m30s`, `e2!` `c2^` `x.` `x&` `x:3`, the octave marks of `deg( )` (`5'` `1,`), and the
two-mark comparisons `==` `!=` `<=` `>=`. A space where they must touch is an error that says so. Everywhere else
whitespace is free.
`%` is the percent mark when it follows a digit with no space (`50%`) and the modulo operator otherwise: write
`$i % 4` with spaces.

The header lines of a `.beatscript` file are comments to the language, all but the version line, and have a grammar of
their own (F14):

```text
#! beatscript 2                               the version line ("Versions"): BeatScript 2; none, or #! btw 1, is BeatScript 1
#! genre: techno                              one genre of the BeatScript ontology: a name, genre:techno, bw:Techno or an IRI
#! tags: acid, four on floor, drop            terms of it, by local name, label or IRI
```

The productions were read against the parser (where a long note and the parser disagreed, the parser won), and the
examples of the feature sections below are lines of the programs in `examples/beatscript/`, every one of which
`crates/djlive/tests/examples_beatscript.rs` compiles.

### Which target takes which action

| action | no target | `deck A:` / `deck B:` | `master:` | `control:` |
|---|---|---|---|---|
| a part line (`kick x ~ x ~`) | — | yes (no time word) | — | — |
| `swap pairs`, `move`, `load pair`, `queue`, `crossfader` | yes | — | — | yes |
| `load SPEC`, `play`, `stop`, `sync`, `mute`, `unmute` | — | yes | — | — |
| `filter`, `delay`, `reverb`, `crush`, `resonance` | — | yes (that deck's insert) | yes (the Master's) | — |
| `drive` | — | yes (that deck's drive) | yes (both decks' drive) | — |
| `level`, `low`, `mid`, `high` | — | yes (the strip) | `level` only (the master level) | — |
| `tempo` | yes (a place, not a repeat) | — | — | yes (with a time word) |
| `duck`, `mix` | yes (they name their decks) | — | — | yes |
| `sing "words" on melody` | yes (the only deck that has the role) | yes | — | — |
| a `def`'s call | yes (the deck is an argument) | — | — | — |

A line that puts an action on a target that does not take it is an error that says which targets do. `control:` is the
control deck ("The control deck"): a head in the place of a target, for the engine calls only.

## Patterns: mini-notation

A pattern is one **cycle**, one bar. Its terms share the bar equally, so the step count sets the grid:

```text
kick x ~ x ~            4 terms: quarter notes, hits on 1 and 3
kick x x x x x x x x    8 terms: eighths
snare ~ x ~ x           hits on 2 and 4
hihat x*8               eight hits, one an eighth
hihat [x x] x [x x] x   groups: two hits squeezed into one slot
bass c2 ~ [c2 eb2] g2   notes
bass 1 ~ 3 5            scale degrees of the key
bass 1 ($i + 1) 5       a degree computed (inside a repeat or a def)
melody <c4 e4 g4>       alternation: one a bar, then round
kick e(5,8)             euclid: five hits over eight steps
kick e(3,8,2)           the same, rotated by two
harmony [c3,e3,g3]      a comma inside [] stacks: three notes at once
hihat x*8?0.7           each of the eight hits kept with chance 0.7 (seeded)
bass choose [c2 e2 g2] weights 3 1 1    one of the three a bar, c2 three times as often
melody deg(1 b3 5' ~)   degrees with accidentals and octave marks, one slot
harmony <I V vi IV>     Roman numerals: a chord of the key a bar (bare, one a quarter); a section's own `key` re-reads them
hihat/12 x x ~ x        a cycle of twelve sixteenths: four terms of three steps, drifting against the bar
bass c2~3 e2            elongate: c2 ~ ~, the step takes three slots
hihat x*8?              a bare ? is ?0.5
```

- `*n` plays its term `n` times in its slot (`n` is 1 to 64, and may be `(arithmetic)`). `<a b>*2` is `a b` inside one
  bar (the alternation advances twice).
- A pattern with an alternation is as many bars long as it needs (the least common multiple of the deck's
  alternations, at most 64 bars); the net is that long and loops. A cycle of its own (`hihat/12`) or chance lengthens
  it too ("How long a program can be").
- The grid is sixteenths: a term off it (three or five in a bar) snaps to the nearest step, with a warning
  (`x x x`: steps 0, 5, 11).
- A hit plays the drum's note in a drum part, the key's root in the part's octave in a tonal one. A note plays itself, a
  degree that step of the key's scale (`1` the root, `8` an octave up), a Roman numeral a chord of the key (`harmony`),
  its bass tone (`bass`, `melody`) or the chord a note at a time (`arp`).
- Default octaves: bass 2, harmony 3, melody 4, arp 4. `oct=N` sets it.

### Generators

| call | does |
|---|---|
| `e(k, n)` / `e(k, n, rot)` / `euclid(...)` | `k` hits over `n` steps (Bjorklund, as `djrust::gen::euclidean`), rotated by `rot`. As a term it takes one slot, so `kick e(5,8)` is a bar of eight steps and `kick x e(3,8)` puts the eight steps in the second half. `0 <= k <= n`, `1 <= n <= 64` ("Euclidean rhythms and polymeter"). |
| `markov(key, density)` / `markov(density)` / `markov(key, density, seed)` | a Markov melody of one bar in the key's scale (`djrust::gen::markov`). The key is the program's `key` (or the section's) when left out. `density` 0..1. Same seed, same notes: the seed is, when left out, taken from the text of the part. |
| `arp(chord, mode, n)` | the chord's notes one at a time over `n` steps (default 8) in `up`, `down` or `updown`. Either takes `shift=n`: its notes `n` key degrees up (-21..21; `up n` writes it). |
| `chord(chord)` / `chord(chord, octave)` | the chord's notes together. A chord is a symbol (`am7`) or a numeral (`V7`). |
| `choose [a b c] weights 3 1 1 [seed=4]` | one option a bar, picked by a hash, by weight (not a call: it has its own words, "Seeded chance"). |

### Parameters

`name=value` after the pattern, in any order:

| param | value | does |
|---|---|---|
| `vel=90` / `vel=70..100` | 1..127 | velocity. A range gives each hit its own, by a hash of its place: the same text, the same velocities. |
| `oct=2` | -1..8 | the part's octave. |
| `len=3` | 1..16 | note length in steps (drums 1, tonal 2). |
| `inst=sub-bass` | a name | the instrument. |
| `seed=7` | a whole number | the seed of the part's generators (and of its chance terms, unless they say their own). |
| `glide=70` `decay=900` `pitch-drop=7` | ms, ms, semitones | tonal: `^` bends into the next note; each note's tail; starts 7 semitones high. On `inst=acid-bass` a `^` slides without `glide=` (60 ms) and `decay=` is its filter's sweep. |
| `cutoff=400` `res=80` `env=60` `accent=90` `drive=20` | Hz 20..12000, then 0..100 each | tonal, the acid voice's knobs: the filter's floor, its resonance, how far each strike opens it (100 is 6 octaves), how much an accent `!` does, how hard the oscillator drives the filter. Defaults 300, 72, 45, 80, 0. On any other voice they are ignored, with a warning. |
| `voice-lead` / `voice-lead=off` | on or off | a `harmony` (or `arp`) part of chords: each chord after the first is voiced for the least motion from the one before; bare, it is on. |

Every number of a param may be arithmetic (`vel=(60 + $i * 4)..100`, `oct=3 - 1`).

A **range** `a..b` is *any value from a to b*, only ever for a parameter of a part. A **ramp** `a -> b` is
*moving from a to b*, only for a control change, with `over N bars` (or beats).

### Settings

`tempo 126` (BPM, 40..300), `swing 0.1` (0..1), `key am`. A setting applies to the whole program. `key`
is the default for notes with no octave, scale degrees, Roman numerals, `markov`, and the key tag the share carries.
No `key`: C major. No `tempo`: the decks keep theirs.

The tempo can move. `tempo 100 -> 130 over 8 bars exp` ramps it from the program's first bar and holds the end; a
`tempo` line may stand anywhere a place does (`at bar 16: tempo 128 -> 140 over 4 bars`) and a section may set its own
(`tempo 140` as a statement of its body, from the section's first bar). The bar clock never moves: see "Tempo curves".
A section may also begin with a `key` of its own, which re-reads its degrees and numerals ("Degrees, numerals, a key a
section"). `swing` stays a setting of the whole program.

`swing s` is how late the off-step plays: every second sixteenth of every deck sounds `s / 2` of a sixteenth after its
place. So `swing 0.6` is 0.3 of a sixteenth (37.5 ms at 120 BPM), and `swing 1`, the most, is half a sixteenth, a dotted
3:1 (a triplet feel is about `swing 0.67`). The scale is the beats generator's own 0..100 read as 0..1: `gen@N` and
`seed@2(...)` write a genre's swing this way (jazz 0.6, house 0.2), and it reaches the engine as beats' 60 or 20. It moves when
the notes sound, never where they are: the steps, the bar lines, a pattern's text and its net stay as written. The arrange op
`swing amt` ("Arrange mutations") is another thing: it rewrites a pattern, moving every second step `amt` of a step later on the
sixteenth grid (a step of two sixteenths or more), so it changes the notes and the net, and only that pattern's.

## Control

Every control line compiles to calls on the **control-language registry** (docs/CONTROL.md), the same
calls `djctl app` and the `djapp_*` MCP tools make, so a line can be checked with `describe`:

| line | registry calls |
|---|---|
| `swap pairs with echo_out` | `autodj {action: set, style: echo}`, then `autodj {action: next}`: the loop-set pair steps to the next group on the next bar line, with that move across the handover (`filter_exit` → style `filter`, `echo_out` → `echo`, `loop_roll` → `roll`; none: `auto`) |
| `move bass_swap on B over 8 bars` | `moves {action: arm, kind: bass_swap, deck: B, bars: 8}` |
| `load pair dnb:2602 group 2` | `loopsets {action: load_pair, spec: dnb:2602, group: 2}` |
| `deck B: load techno:42` | `deck {action: load, deck: B, spec: techno:42}` |
| `queue play` / `queue jump 3` | `queue {action: play}` / `queue {action: jump, n: 3}` |
| `deck A: play` / `stop` / `sync` | `deck {action: play, deck: A}` / `stop` / `sync` |
| `deck A: mute` | `mixer {deck: A, mute: true}` |
| `master: filter lp 2k` | `inserts {action: on, target: Master, effect: filter}`, `inserts {action: set, target: Master, param: filter.sweep, value: v}` (v from the hertz: the knob's own curve, below 0.5 low-pass, above 0.5 high-pass) |
| `deck A: filter hp 400` | the same with `target: A` |
| `master: filter off` | `inserts {action: on, target: Master, effect: filter, on: false}` |
| `master: delay 0.4` | `inserts {action: on, ... effect: delay}`, `inserts {action: set, param: delay.mix, value: 0.4}` |
| `master: reverb 0.3` / `crush 0.5` / `drive 0.6` | the same with `reverb.mix` / `crush.mix`; `drive` is `fx {action: set, param: drive, value: 0.6}` (the FX unit's drive: with `deck A:` it is `deck: A`, with `master:` no deck, which is both; 0 is the unit at neutral) |
| `master: level 0.8` | `mixer {master: 0.8}` |
| `deck A: level 0.7` / `low 0` / `mid` / `high` | `mixer {deck: A, volume: 0.7}` / `{deck: A, low: 0}` ... |
| `crossfader 1` | `mixer {crossfader: 1}` |
| `duck B under A.kick 6db` | `trick {action: on, deck: B, kind: pump, source: A, amount: 0.499, div: 16}` (put back with `trick {action: off, ...}`) |
| `mix { A level 0.7, A eq low -3 }` | `mixer {deck: A, volume: 0.7}`, `mixer {deck: A, low: 0.4375}`; in a section the mixer is read first with `mixer {}` and put back when the section ends |
| `deck B: sing "words" on melody` | `vocal {action: create, lyrics: ..., notes: [...], ...}`, then `vocal {action: to_row, play: true}` (the browser: a sung part in deck B's net, docs/beatscript/phonemes.md) |
| `at bar 16: tempo 128 -> 140 over 4 bars` | no call of its own: a tempo is an event of the program's tempo map; the runtime sets the decks' tempo (`deck {action: tempo, deck: A, bpm: ...}`) each service pass |
| `macro riser on A over 8 bars` | `effects {action: fire, target: A, id: riser, bars: 8}`: an entry of the DJ macro catalogue by name ("Macros", below), made a beat ahead so it starts on the line's bar |

A **ramp** `filter lp 2k -> 8k over 8 bars` sends the value afresh on every service pass, from the bar
it starts on to the end of the span, so it moves smoothly inside the bars, and it ends where it says. A
plain `filter lp 2k` sets it once. `over` with no `->` is an error (nothing to move). A ramp goes straight unless it
names a shape (`over 8 bars exp`), and a value can also keep moving, as an `lfo` ("Shapes and lfo").

**`drive`** is the saturation of a deck's sound (`deck A: drive 0.6`) or of both decks' (`master: drive 0.6`), 0 to 1, said, ramped and
swung like `reverb` (`master: drive 0.2 -> 0.8 over 8 bars exp`, `deck A: drive lfo sine 2 bars center=0.5 depth=0.3`); `for 4 bars`
puts it back to 0. It is no new effect: it is the engine's FX unit's drive, the master chain's distortion that the Effects window's
Drive knob turns, the waveshaper `y = (3 + k) x 20° / (π + k |x|)` with `k = 100 * drive`, fully wet whenever drive is above 0 (the
switch fades over 50 ms). **0 is off**: the unit at neutral, which changes no sample, so a program with `drive 0` renders the samples
of the one without the line. Above 0 the curve takes a quiet signal up (a gain of `(3 + k) / 9`, 7 at drive 0.6) and bends a loud one
over: the output never goes past about 0.35 (-9 dBFS) however hard it is driven, so drive adds the odd harmonics (a kick that hums at
a set pitch, a squared acid line, a crunched guitar) and does not add loudness; there is no makeup gain, `level` is the other knob.
The value is held to 0 to 1: `drive 2` is an error that says the nearest, `1`. Parts share their deck's strip, as they share its
filter, so a kick that wants drive alone is a deck of its own (`deck A: kick ...` driven, `deck B:` the rest); `master:` drives each
deck's chain, not one on the sum. The browser, the booth and the phone run the same engine code: the same text, the same samples.

### Time words

Any line can have a time word, then the target, then what it does: `at bar 16 for 4 bars: master: filter lp 800`.

| word | fires |
|---|---|
| (none) | once, on the first bar line after the statement is added or changed (in a `section`: on its first bar); a value that names an input (`knob(1)`) follows it on every bar |
| `every 16 bars:` | on bars 16, 32, 48 ... (never on bar 0) |
| `every 2 beats:` | every half bar, from the program's first bar: at 1/2, 1, 1 1/2 ... of a bar (never at 0). The finest is `every 1/64 bar` |
| `every 4 bars from 8:` | on bars 8, 12, 16 ... (`from` takes a bar, a fraction of one, or a position: `from bar 3 beat 2`) |
| `on phrase:` | on bars 8, 16, 24 ...: every phrase |
| `on next phrase [8\|16\|32]:` | once, on the first bar that is a multiple of 8 (or 16, 32) |
| `at bar 32:` | once, on bar 32 (in a `section`: bar 0 is its first bar, so `at bar 4` is the fifth) |
| `at bar 16 beat 3:` | once, on the last quarter of bar 16. Beats count from 0 like bars: `beat 0` is the bar line, `beat 4` is an error (write `at bar 17`) |
| `after drop:` | once, on the bar the section `drop` ends on |
| `before drop 4 bars:` | once, four bars ahead of the section: the roll that leads into a drop (`before drop 2 beats:` is a half bar ahead) |
| `for 4 bars` / `for 2 beats` | after any of the above: the effect is **put back** after that long (`for 4 bars:` alone is now) |

`on next bar:` is the same as no word. A length is `4 bars`, `1 bar`, `1/2 bar` or `2 beats`; a beat is a quarter of a bar. A time between bar lines fires
when the deck's beat position **crosses** it, once. `at bar 3/2` is `at bar 1 beat 2`. At the top of a program with no sections
`at bar 0` is an error (the first bar after the start is `at bar 1`) but a time inside bar 0, `at bar 1/2` or `at bar 0 beat 2`,
is legal and fires once before bar 1.

`for N bars` puts back what the line did where the verb has an off: a filter, a delay, a reverb, a crush
(`inserts ... on: false`), a drive (turned to 0), `mute` (`mute: false`), `freeze on` / `trick on` / `moves arm` (the verb's pair),
`play` (`stop`), a `duck` (`trick off`) and a `mix` (the mixer's own values). A line with no off (a level, an EQ, a
ramp of a plain value) says so as an error: ramp it back with a second line. `every 16 bars for 4 bars:` puts it back
each time.

A **ramp** `a -> b over N bars` works on any numeric param of any line (`do vocoder set mix=0 -> 1 over 8 bars`,
`master: resonance 0.2 -> 0.8 over 4 bars`), one ramp to a line. `over` with no `->`, or a `->` with no `over`,
is an error.

**Program bar 0** is the bar the program first started on. It keeps counting through every
re-evaluation, so editing a line never moves the schedule of the others. `btw stop` ends the program,
and the next eval starts a new one at bar 0. In a song with sections (below), bars are counted from the
**song's own first bar**, the bar the first section lands on.

In a program with no sections, **bar 0 is the bar the evaluation fell in**, and that decides what the first bar you hear is called.
With nothing playing the music starts on it at once, so it is bar 0 and the first bar you hear. With a deck already playing nothing of
the program sounds on it (a net lands on the next bar line) and the music lands on bar 1: `at bar 1` is the first bar you hear, `at bar
2` the second. The offline render and `btw test` start the music on their bar 0 (nothing was playing there), so a timed line of a
program with no sections, evaluated while a deck plays, lands one bar *later* counted from the first bar you hear in the render than
it did live (`at bar 2` is the second bar you hear live and the third in the render). A song with sections is not affected: both count
from its first section. Programs written before the extensions have always counted this way and it is kept. The one thing counted from
the first bar you hear in both is the tempo: a tempo curve and the `tempo` lines of a program with no sections (the tempo map, a
timeline of its own) start on the bar the music lands on, live and in the render, so the ramp is not a quarter spent before the music
lands (see "Tempo curves"). (The `lands` an evaluation answers, and the bar numbers in `btw get`'s words, count the clock: the bar you
evaluated in is the clock's bar 0, so a program that lands on the next bar says `lands: 1`.)

### Guards

A line can end with `when` and a condition: `deck A: kick x ~ x ~ when $i % 4 == 3`, `at bar $i: master: filter lp 300
when $i % 2 == 0`, `drop(A, 8) when $i != 0`. The condition is read where the line is **expanded** (so inside a `repeat`
or a `def` it sees `$i`, `$n` and the def's `$params`; at the top it is a constant), and a line whose guard is false is
**removed before it is checked or compiled**: a guarded-off line may name things that do not exist in this repetition
(a section, a second `kick`). A guard is a comparison (`== != < <= > >=`) joined by `and` `or` `not`, over arithmetic
and words (`when $deck == A`). It belongs to lines (a part line, a control line, a `do`, a def's call), not to `let`,
`def`, `section`, `tempo` or `key`. On a control line it may name a performed input (`when held(key)`): then it is
decided while the program plays ("Performed inputs"). `when` does not start a pattern step, so `kick x ~ when ...` ends the
pattern there.

### Moves put their strip back

`filter_exit` and `echo_out` end by muting the deck's channel. A `move` from a program restores it: **the
strip is unmuted at the first phrase line after the move has ended**, whatever else happens, so a recurring
`every 16 bars: move filter_exit on B over 4 bars` never leaves B silent past the next phrase. `for N bars`
on a move puts it back after N instead; `btw stop` and an edit that drops the line put it back at once. The
deck's panel says **MUTED** in red while a strip a program plays on is muted, and `btw get` has it in `warnings`.
A `move` takes whole bars (the registry's `moves arm` takes bars): `over 2 beats` on a move is an error that says so.

### The control deck

`control:` is a deck of engine calls, beside `deck A:`, `deck B:` and `master:`. It takes the lines that take no target
(`move`, `swap pairs`, `load pair`, `queue`, `crossfader`, a `tempo` at a place, `duck`, `mix`, `macro`, `do` and its sugar nouns), in
the place a target goes, and sections, repeats, defs, guards and time words sequence it like any line:

```text
section drop 16 bars {
  deck A: kick x x x x | bass c2 ~ c2 ~
  deck B: arp arp(am7, up, 8)
  master: filter lp 400 -> 8k over 8 bars
  control: move echo_out on B
  at bar 12: control: swap pairs with filter_exit
}
every 16 bars: control: freeze phrase bars=2
```

A line on it compiles to exactly what the line with no head does: the same registry calls, the same key (so moving a line
onto it, or off it, fires nothing again), the same nets. The canonical text keeps the head, so `control: move echo_out on B`
and `move echo_out on B` are two spellings with two CIDs; both stay, and no program's CID moved. The control deck never holds
a part or a note (`control: kick x ~ x ~` is an error that says to write it on `deck A:`), a strip's action (`control: filter lp
400`: on `deck A:`, `deck B:` or `master:`), a deck's transport, a `sing`, a def's call or a setting (`control: tempo 140` with
no time word is `tempo 140`). `control` is no reserved word: it is the head only before its colon. The long note is
`docs/beatscript/ext-control.md`, with `macro`'s.

### Macros: the DJ catalogue

```text
control: macro riser on A over 8 bars
at bar 24: control: macro cathedral on B over 2 bars
macro@1 breakdown over 4 bars on A
```

`macro ID on DECK [over N bars]` fires one entry of the engine's DJ macro catalogue, the 39 pads of the apps' Effects window
(`riser`, `tape_stop`, `breakdown`, `cathedral`, `ping-pong`, `feel-build` ...: the `macros` MCP tool lists them), on a deck: the
registry's `effects fire`, so the booth, the window and the program fire the same thing. It is a line of the control deck, and a
line with no head. `on DECK` is required (the focused deck is the person's, not the text's); `over N bars` takes 1 to 64 whole bars,
and with none the entry runs the window's own length (1 bar, 2 for a sweep). A transition runs its own length: the riser rises over
its `over` (4 bars with none), a tape stop, a spin back or a reverb freeze is held `over N bars` and let go (1 with none), and a cut
or a double drop takes no `over`. The engine starts an engagement on the deck's next bar line, so the call goes a beat ahead of a
bar-line time and the macro starts on the bar its line names (`at bar 24:` is sent at bar 23 beat 3); at the song's very first bar
there is no beat before it, and it starts on the next.

**Versioned.** `macro` is `macro@1`: a frozen copy of the catalogue as of the engine this repo pins, each id with its category,
family and length, never edited, so a stored program's `macro riser` means what it meant. A later catalogue is `macro@2`, a table
beside it; the canonical text writes the version only when it is not 1 (`macro@1 riser` is `macro riser`). An unknown id is an error
with the nearest (`macro rizer`: did you mean `riser`?), and an id only a later version has says which. `render` and `listen` play
a macro line (`listen` names each in `macros`, with the bar it starts on and whether the engine played it); a browser tab does not,
and lists it as not played here. `macro` is no reserved word.

An `arrange { }` switches the control deck pass by pass: `~ control off` (its lines do not fire in the sections from here on),
`~ control on`, `@ control off` (the next section only), `~ control freeze off` (the lines of one call), `~3 control off` (every
3rd pass). The head is what a switch reaches: `move echo_out on B` with no head keeps firing ("Arrange mutations").

## The whole surface: `do`

```text
do <noun> [<verb>] [<positional text>] key=value ... [over N bars [shape]]
```

is any call of the control language (docs/CONTROL.md), checked against the registry the same `describe`,
the MCP tools and CONTROL.md come from. The noun and verb must exist (an alias works; `do vocoer on` says
`did you mean vocoder?`), every key must be a param of the noun, with the right type (`bands=2.5` is "a whole
number"), one of the allowed words where the registry lists them (`deck=C`), and what the verb requires must
be there (`do vocal create bars=2` needs `lyrics`). The errors give line and column, what would have been
accepted and the nearest name. The `btw` noun's verbs that act on the program that runs (`eval`, `stop`, `load`, `open`,
`replay`, `perform`) are errors in a program, at `check` (`at bar 4: do btw stop` did nothing: the runtime holds the program while its
lines run, so the call acted on an empty placeholder); the others (`check`, `fmt`, `save`, `list`, `jsonld` ...) work. A bare string after the verb fills the verb's first required text param:
`vocal create "robots on the road" bars=4`. Values are numbers (`2k`, `50%`, arithmetic: `bands=2 + $i`), words
(`on` / `off` are booleans where the param is one; `dnb:2602` specs; `param=delay.mix`), `"strings"`, `[lists]`,
`a -> b` ramps (with a shape after `over`: `mix=0 -> 1 over 8 bars exp`) and `lfo` values
(`value=lfo tri 1 bar depth=0.2 center=0.3`). A ratio such as `ratio=1/4` is the registry's word `1/4`; `ratio=(1/4)` or
`1/4 * 2` is arithmetic.

The sugar is the same line with no `do`: `vocoder`, `vocal`, `voice`, `trick`, `freeze`, `loops`, `clips`,
`wheel`, `td3`, `pd705` (the `kits` noun), `effects`, `aux`, `automix`, `record`, `mashup`, and `key` when a verb
of it follows (`key detect deck=A`; `key am` is the setting).

```text
every 16 bars: freeze phrase bars=2
at bar 8 for 4 bars: trick on deck=A kind=stutter
vocal create "robots on the road" key=am bars=4
do vocoder set mix=0 -> 1 over 8 bars
do td3 write confirm=true
```

### The wheel

The `wheel` lines play windwheel's tonewheel instrument (the Wheel window, on the Windwheel input strip). Each is the
sugar for `do wheel <verb>`, checked against the registry like any other call:

```text
wheel load preset=NAME          # an instrument, by its id from `do wheel presets`
wheel key root=C transpose=2    # the key every note is re-wound for: root C, F#3, Bb2 ..., transpose -24..24
wheel sequence notes="C E G B"  # the notes the path plays, in order; without notes, the preset's own
wheel motor on=true speed=2     # a cord instrument's motor: on / off, speed in m/s (0..25)
wheel level db=3                # the wheel's level, dB -20..40
wheel play
wheel pause
```

A wheel line takes a time head like any other (`at bar 16: wheel play`). The browser engine on beatscript.app plays the
wheel too, the same instrument as the booth: the wheel goes into the master after the decks, so `master:` filter, reverb
and level reach it.

## Functions and structure

Everything here is **total**: there is no recursion and no loop without a written-out count, so `check`
always proves a program ends, and the same text always gives the same CIDs. (Arithmetic is part of that: an expression
is a tree walk, evaluated once when the program is read.)

```text
let kicks = x ~ ~ ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~       # a pattern: used where a pattern goes, as a group
let late = 8                                       # a value: put in place of every later use
let root = c2

def groove(deck, bars) {                           # a macro, expanded at each call
  deck $deck: kick kicks | bass root ~ root ~
  repeat $bars every 1 bars {                      # a literal count; $i is the copy's index
    master: filter lp 300 -> 2800 over 1 bars
  }
}

section drop 16 bars {                             # lines in it count from its first bar
  master: filter off
  groove(A, 16)
}
arrange intro build drop breakdown drop2 outro
```

- **`let name = ...`** at the top. One token (`8`, `c2`, `am`) is a value put in place of every later use of
  the name (so `at bar late:` and `bass root` work, and the canonical text shows the value). More tokens are a
  pattern, used as `bass kicks`, in `<a b>` alternations, in `[a, b]` stacks and as the pattern a transform
  takes. **Arithmetic** that is the whole value is a number: `let n = 2 * 4` is 8 (an earlier scalar may be used in
  it: `let m = n + 1`); `let r = 3*2` is the number 6, the degree 3 twice is `let r = [3 3]`. A section expression
  names a composition (`let verse = stack(groove, wash)`). A name cannot be a
  word of the language, a note, a chord, a key or a Roman numeral (`let kick`, `let a` and `let vi` are errors).
- **`def name(a, b) { ... }`** is a macro: its body is lines, and `$a` is where an argument goes. A call
  `name(x, y)` is a line, with `at bar N:` / `after X:` / `before X N bars:` or none. The body is read when it
  is called, so an error in it says where in the def and which call. **A def that calls itself, directly or
  through another def, is an error** (`ping -> pong -> ping`) found before anything runs. A def
  may declare a **boundary** (`def walker(oct) in: chords, key out: bass { ... }`) and be glued to others with `wire`:
  typed ports, below.
- **`repeat N { ... }`** with `N` written out (1 to 64). `repeat 8 every 2 bars { ... }` shifts copy `i` by
  `i * 2` bars; `$i` and `$n` are the index and the count, and are put in place before an expression is read
  (`at bar $i * 2 + 1`). Written out, a program is at most 4096 lines ("Bounds on the text"). A def's body may have a
  `repeat` of its own: its `$i` and `$n` are the repeat's.
- **`when` guards** end a line (`drop(A, 8) when $i != 0`, `deck A: kick x ~ x ~ when $i % 4 == 3`): the line is kept
  in the copies where the condition holds and removed, before it is checked, in the others. They see `$i`, `$n` and a
  def's `$params` ("Guards", above).
- **`section name N bars { ... }`** and **`arrange a b c`**: the song is the sections in that order (a section
  may be named twice; without `arrange`, in the order written). A section is 1 to 256 bars, and its body is
  statements, one to a line (`{` ends the line it is on; a block of a `def` or `repeat` may be on one line, a section's
  may not). Lines outside every section apply to the whole
  song, a section's own part of the same role wins, and a deck a section does not use is stopped there. After the last
  section the last nets play on. `section` and `arrange` belong at the top, not inside another block. `arrange` also takes `stack(...)`,
  `then(...)` and `silence` ("Composition: stack and then"). A section may open with a `key` of its own and may set a
  `tempo`.
- **`import "name"`** or `import zQm...` brings the **lets and defs** of a saved program, with the ports of its defs
  and its `wire` lines (not its sections, its composition lets or its assertions). The canonical text
  holds the **CID**, not the name, so an import is pinned: the same program text is the same program for good
  (docs: Canonical form). An imported name cannot clash with one of the program's. **Where it finds the program** does not change
  what the program means: the language never fetches, so a program is checked from its text and the programs it is given: the
  ones a share link carries (below), then the host's own (the desktop's and the phone's saved programs and the links they opened, the
  programs beatscript.app keeps, `beatscript check --with FILE`, the page's fetch by CID). One no host has is an error on its line
  (`z...ukZ is not here: this program draws on it, and check needs its text`), the same words everywhere. The web takes a CID; a
  name needs a snippet store, which only the desktop and the phone have.

### Ending

A program with a **timeline** (`arrange`, or sections in order) **ends**: after its last section the decks stop on the
bar line, what the lines still hold changed is put back (a strip a move muted, a filter), the ramps end, and the
window and `btw get` say **ended** (`ended`, `ended_at_bar`). Nothing wraps round to bar 1. A section that wants an
echo or a fade says so in its own lines (`move echo_out on A over 8 bars` in an 8-bar outro ends where the song does).

| line | does |
|---|---|
| `end` | the same as the default: after the song's last section |
| `end after outro` | where the section `outro` ends (the song stops there, later sections never play) |
| `end at bar 64` | on bar 64 of the song (of the program, with no sections) |
| `loop` | at the song's end it starts again from the first section, its `at` lines firing again each lap |
| `arrange set: drop wubs then close` | a named arrangement that goes on to another one (below) |
| `arrange set: drop wubs loop` | a named arrangement that repeats itself for ever: the song's lap starts at it, not at the first section |
| `launch drop on pad(2)` | a hit of pad 2 brings in the section (or arrangement) `drop` on the next phrase line, where it loops; `next bar` or `next phrase 16` set the line |
| `arrange close: outro end` | a named arrangement that ends the song |
| `start set` | the arrangement that plays first (default: the first written) |

`arrange set: drop wubs loop 4` plays four passes in all, then goes on to the next arrangement written (the song ends after the last one).

A program with no sections plays on by itself (its patterns loop) until it is stopped or `end at bar N` says where.
A song either ends or loops, not both, and `end` is said once. Evaluating another program after the end starts fresh.

`btw stop` and the end put back what the program took: a strip a line still holds changed, a **tempo curve's** tempo (the
tempo each deck had when the program was applied, read before its nets landed, goes back to it: the words say "tempo put
back", and a deck whose tempo the host could not say is left where it is), and a **`sing`** line's voice (the clip it put on
the loop row is stopped with `loops stop`; so it is when an edit removes the line, and where a lap of a looped song turns).

### Several arrangements

A song may have several **named arrangements**, each saying how it ends, so one program can have an intro that plays
once, a section that loops, and an outro. `arrange a b c` with no name stays the one plain arrangement (a program may
have either that or named ones, not both), and every program written that way keeps its CID.

```text
arrange open: intro build then set
arrange set: drop wubs build2 peak loop
arrange close: outro end
start open
```

- An arrangement's ending is `then NAME` (go to that arrangement), `loop` (play itself again for ever), `loop N` (N passes in
  all, 1 to 256, then on as if nothing was said), `end` (the song stops), or nothing: on to the next arrangement
  written, or the end of the song after the last. The first arrangement written plays first, or the one `start` names.
- The flow is a small deterministic walk, unrolled when the program is read: `open` then `set`, then `set` again. A song
  with no `loop` (and no `then` cycle) is finite and has a length (`song_bars`, `seconds` in `check`). A `loop`, or a
  `then` cycle with no `end`, makes it infinite: it goes back, at its end, to the bar where the repeating arrangement
  first began (`flow.loop_from_bar` in `check`), not to bar 0; `loops` is true. `end at bar N` and a top-level `loop` are not
  said with named arrangements (an arrangement says how it ends).
- `check` reports the `flow`: the arrangements and their endings, the `order` they play in, where the lap `loop_from`,
  whether a `then` cycle makes it `cycle`, and the `unreachable` arrangements (a warning, they never play). A `then` to a
  name that is not an arrangement, an unknown section, a second `start` and a repeated arrangement name are errors.
- Every consumer plays it the same way, since it is one timeline: the Live runtime, `beatscript render` (an infinite flow renders
  one pass: the arrangements in order up to the end of the first lap, as `loop` always did), the browser wasm and
  beatscript-serve's `/api/render`. At most 2,048 plays of arrangements, and the song's usual 4,096-bar cap, apply.
- One named arrangement may be an `arrange { }` with mutations: `arrange set: { drop ~ acid rot 2 drop } loop 2` (one
  item a line). See [ARRANGE.md](ARRANGE.md) "A named block".

### Scenes and launch

```text
section verse 8 bars { ... }
section drop 8 bars { ... }
section breakdown 8 bars { ... }
arrange verse drop breakdown verse
loop

launch verse on pad(1)
launch drop on pad(2)
launch breakdown on pad(3) next phrase 16
```

A **scene** is a section (it plays on its own and loops until the next launch) or a named arrangement (it plays as `start NAME` would, from
its first section, and ends as it says: `then`, `loop`, `end`). A section and an arrangement share a namespace, so `launch drop` is never
ambiguous. `launch NAME on pad(N)` says which pad of the controller brings which scene in, so the written song is the song you hear until
a hand comes to the pads.

- **A hit queues the scene, and it lands on a bar line:** the next phrase line, **8 bars counted from the song's first bar** (it is never
  counted again from a launch), unless the line says `next phrase 16` or `32`, or `next bar`. The last hit before the decision wins. The engine
  is given the first three quarters of the bar before a line to load the scene's nets, so a hit in the last quarter of that bar waits for
  the next such line; `btw get` says which: `bar 21: drop queued, lands bar 24`. A pad held down is one hit.
- **It lands as a lap of the song** that begins at the scene instead of at the loop point: what the lines that leave hold is put back, ramps
  and lfos end, the scene's lines count from its bar 0 (lines outside every section too, as in a lap of a `loop`), and its tempo and key are
  the scene's. Nothing carries over: an `evolve` and arrange mutations begin from the text as written.
- **A song that has ended** is not a stopped pad: a hit after the end starts its scene at once. A host with no controller (the page, the phone,
  `beatscript render`) never sees a hit, and plays the song as written.
- **Top of the program only;** one pad launches one scene, and one scene may be launched from two pads. `check` lists them
  (`launch: pad(1) verse, pad(2) drop, ...`), does not warn that a scene "never plays", and compiles each scene on its own (the program with the song
  replaced by it, `launches` in the report). A line that counts from a section that is not in the scene (`before drop 2 beats:` in the verse,
  when the scene is the verse) has nothing to count from there: it does not fire in the scene, and `check` says so where it is.
- **Editing while a scene plays** lands on the scene; a scene taken out of the program sends the song back to its first bar. The take logs the
  scenes brought in (`"launches": [{"bar": 24, "scene": "drop"}]`, said only when there are some), and a replay brings them in on the same bar
  lines, whatever the pads say.

Long note: `docs/beatscript/ext-perform.md`, "Launch".

### Evolving

`evolve` is a program that applies transforms to itself over time: the variation inside a section or a loop, where
`arrange` stays the authored structure.

```text
section drop 32 bars {
  deck B: melody [a4 c5 e5 g5] | bass [a2 ~ e2 ~]
  deck A: kick x ~ x ~ | hihat x*8
  evolve every 8 bars {
    melody: rotate 1 | transpose <0 5 7 0>
    hihat: thin 0.1 seed=3
    bass: invert when gen % 2 == 1
    stop when gen == 3
  }
}
```

**The model.** The parts as written are **generation 0**. Every `every` bars the next generation is made from the
one before by the rules, so generation n is `f_n(...f_1(gen 0))`; with a rule that does not mention `gen` (no
`when`, no `<a b c>`) that is `f^n`, the composite iterated: transforms are endomorphisms (a part to a part of the same
shape; see "Categorical notes"). A rule is `target: transform | transform ... [when guard]`, the
target a role (`melody`, on every deck that has one) or `deck A` (every part of the deck), and the transforms are applied
in the order written. They act on the **realized hits** of the generation before (a step in the bar and a note), not on
the text, so a `thin` takes away what the last generation still had.

| transform | does |
|---|---|
| `rotate n` | every hit `n` steps (sixteenths) along its bar, wrapping |
| `reverse` | the steps of a bar backwards |
| `transpose n` / `octave n` | pitches up `n` semitones / octaves (tonal parts; a drum's note is the drum) |
| `invert` | pitches turned over about the part's first note (tonal parts) |
| `thin p [seed=s]` | drops the share `p` of the hits, picked by a hash of (seed, generation, bar, step, note): the same seed, the same hits |
| `fill p [seed=s]` | each step of a bar with no hit on it takes, with chance `p` (a hash of seed, generation, bar and step), a copy of the hits of the step before it that has some; a bar with no hits stays empty |
| `slow n` | the first `1/n` of each bar played over the whole of it: a hit at step `s` moves to `s*n` (its length with it), what would fall past the bar's end is gone (`n` 1 to 16) |
| `fast n` | each bar played `n` times in the time of one: a hit at `s` lands at `s/n` and again every `16/n` steps (`n` 1, 2, 4, 8 or 16) |

The transforms act on realized hits, so they are not the arrange mutations' operators (which rewrite a pattern's text): a
mutation operator an evolve does not take (`drift`, `ratchet`, `morph` ...) is an error that says it belongs to `arrange { }`.

An argument `<0 5 7 0>` takes one value per generation, round (generation 1 the first). A guard is `gen == 3`,
`gen % 4 == 3`, `gen >= 8` (also `!=`, `<`, `<=`, `>`): the rule runs in the generations it is true for.

**Total, and the bound.** `stop when gen == 12` (or `>= 12`) or `for 12 gens` says the last generation (at most 256): it plays on
to the end of its section. It is **optional**, because the evolution is finite anyway: a part is a finite set of hits, so the
sequence of its generations (with the rule's own clock, the lengths of its `<a b c>` and the moduli of its guards) must come
round to a state it has had. In a section with no `stop` the section's length is the bound (as many generations as fit). In a
program with no sections, no `stop` means the **orbit**: the generations up to where it comes round, played through and ended
(over 256 generations of orbit is an error that says so and asks for a `stop`). Generations that would start after the section
is over are a warning ("generations 4 to 12 never play"). A program with no sections has its own timeline of `last + 1`
generations of `every` bars and **ends** when it has played them (`loop` starts it again); `at bar` lines in it count from its
first bar. In a song, an `evolve` belongs inside the section it varies (one to a section).

**Tail and period.** Because a rule acts on each part alone, the program's orbit is the parts' together: the longest **tail** (the
generations before a part settles into its cycle) and the **period** (the least common multiple of the parts' periods). `check`,
`eval` (`evolve`) and `djctl btw check` print it when what plays comes round to it (the tail and one period fit before the
`stop` or the section's end, or a program of its own plays its orbit through):

```text
evolve in drop: tail 34 generations (272 bars), then it cycles: period 4 generations = period 32 bars ≈ 1m02s at 124
```

(that is the example above with its `stop` taken out and its section long enough to play the orbit through: the melody climbs until its notes reach the top of the MIDI range, the
thinned hi-hat runs out of hits, and what is left comes round every four generations. A rule whose parts come round in 7, 11 and
13 generations of 8 bars has a period of 1,001 generations, 8,008 bars, 4h18m at 124.)

When the `stop` or the section's end comes first, they say what plays instead, and that it does not cycle:

```text
evolve in drop: generations 0 to 5 in 24 bars, then it stops at gen 5 (its `stop`); no generation repeats, so it does not cycle
evolve in drop: generations 0 to 2 in 6 bars, then the section ends; no generation repeats, so it does not cycle
```

`thin` picks hits by a hash of the generation number, so a part it acts on is not a function of its state and the rule's
clock: its only provable fixed point is the empty part (which every transform leaves empty), and a thinned part's orbit is
found when it has no hits left (or not found, said so, if that is not within 20,000 generations). `fill` is random the same
way; with no `thin` against it, its fixed point is the part with a hit on every step of every bar that has one, found when a
generation leaves it as it was under rules with no clock of their own (no `when`, no `<a b c>`).

**Any generation, from the rule.** `btw generation {gen: N}` (and `djctl btw gen FILE N`) answers the CID of generation N
computed from generation 0 and the rule alone: no chain stored, and a part that has come round its cycle is brought back to where
it is in it, so generation 1,000,000 costs the length of the orbit, not a million steps (a test walks every step by hand and
demands the same CID).

**CIDs and lineage.** A generation is compiled as a segment of the timeline, so the runtime swaps its nets in on the bar
line like any section's. Each generation has a **CID of its nets** (content only: `djrust.live.gen` over the deck
to net-CID map) and records the CID of the generation it came from: the chain a fork's `forkedFrom` is made of. A
rule that changes nothing (`transpose 0`, a fixed point) gives **the same CID every generation**, each forking itself.
`eval` answers the whole chain (`lineage`: scope, gen, cid, forkedFrom, start bar, bars) and `btw get` and the window say
which generation is playing. The share link carries generation 0 and the rule (it is the program), so opening it
reproduces every generation, CID for CID. (The evolved generations are not saved as programs of their own: they are
a function of the one that is.)

Blocks are written with braces, like every other block, not with the colon and indent of the first sketch.

### Pattern transforms

Terms like generators; a pattern is given as `[...]`, a `let` name, or another transform:

| call | does |
|---|---|
| `transpose(p, n)` / `octave(p, n)` | `n` semitones / octaves up (a hit in a tonal part is its root first) |
| `reverse(p)` / `rotate(p, n)` | the steps backwards / moved `n` steps along |
| `every(n, p)` / `every(n, p, q)` | `p` on every `n`th bar (the 4th, 8th ...), `q` (or a rest) on the others |
| `degrade(p, a)` / `degrade(p, a, seed)` | drops the share `a` of the hits: seeded, so the same text is the same hits |
| `slow(p, n)` / `fast(p, n)` | `p` stretched over `n` bars / played `n` times a bar |
| `humanize(p, seed)` / `humanize(p, seed, amount)` | velocities wander by up to `amount` (default 10), seeded |
| `maybe(p, prob)` / `maybe(p, prob, seed)` | keeps each hit of `p` with chance `prob` (0 to 1): `p` with `?prob` on every hit |
| `rests(p)` / `rests(p, q)` | a hit on each step where `p` rests / `q` kept there |
| `ply(p, n)` | each step of `p` `n` times in its slot (`*n` on each) |
| `palindrome(p)` | `p`, then `p` backwards: `<[p] reverse(p)>` |
| `iter(p, n)` | `p` a `1/n` of its steps later each bar |
| `off(p, n)` / `off(p, n, k)` | `p` and a copy `n` steps later (`k` semitones up) |
| `poly(p, q ...)` / `poly(n, p, q ...)` | each walked `n` steps a bar (the first's count when left out), stacked |

Marks touch their step: `c2!` an accent (velocity +27), `x.` a ghost (40%), `e2^` a slide (held into the next
step; with `glide=` it bends), `x&` a flam, `x:3` a ratchet (3 hits in the sixteenth, 2 to 8). Not `!` with `.`.

### Expression marks

The marks say how a hit is played, the tonal params `glide=`, `decay=` and `pitch-drop=` shape an 808, and
`cutoff=` `res=` `env=` `accent=` `drive=` turn the acid voice's knobs. A
program that uses none of them is the program it was: same canonical text, same nets, same CIDs
(`crates/beatscript-api/tests/cid_pins.rs` pins every program in the repo).

- **Ghost** `x.` `c3.` `[x x].`: the hit at 40% of the velocity it would have had (rounded), after `vel=` and any
  accent. A `.` is a ghost only touching its step with no word after it; `A.kick` is still a deck's part.
- **Flam** `x&`: a grace hit at 45% of the velocity on the step, and the hit itself 22 ms after it.
- **Ratchet** `x:N`, N 2 to 8: the hit N times, evenly, inside its own sixteenth step, at its velocity. A tonal
  ratchet's strikes are each that share of the step long. `hihat x*16` with `x:3` and `x:4` in it is a trap roll;
  `x:3*2` is two ratcheted hits in the slot.
- **Glide** `glide=N` (ms, 1 to 2000) on a tonal part: a slide `^` no longer only holds its note, the voice sounding
  it bends to the next note over N ms instead of striking again (legato, a 303 or an 808 slide). The next note is
  the next step that has one; the last step's slide glides into the first when the net comes round. Without
  `glide=`, `^` holds as it always has, except on the acid voice (below), which slides by itself in 60 ms.
- **Decay** `decay=N` (ms, 10 to 10000) on a tonal part: each note's tail is N ms whatever `len=` says (the time
  its level takes to fall to about a third), so a one-step 808 rings on. On the acid voice it is the 303's decay
  knob instead: how long the filter's sweep takes to fall; the note's length gates its level.
- **Pitch drop** `pitch-drop=N` (semitones, 1 to 24) on a tonal part: each note starts N semitones high and falls
  onto its pitch in 50 ms, an 808's punch. A gliding note glides instead.

**The acid voice** (`inst=acid-bass`, a saw, also called `inst=acid`; `inst=acid-square`, a square) is a 303-style voice
made in the engine (no samples): an oscillator into a resonant four-pole (24 dB) lowpass that an envelope sweeps on each
note, with a soft output stage. It plays the marks as a 303 does. An accent `!` is louder, opens the filter further and sweeps it shorter, and accents in a row build on each
other (the engine reads the accent from the velocity: above 110 in proportion, all of it at 127, so a `vel=` up there is
accented too). A slide `^` carries the voice into the next note without striking it again, the last step's into the
first: in 60 ms, or in `glide=` ms (the note it leaves says `slide` on its binding, so its gate stays open). A note that does not slide lets the one
before go (one note at a time, as a 303; a chord's notes each sound). Its knobs are part params, each on every note's
binding only when set (`cutoff`, `res`, `env`, `accent`, `drive`), so a part without them is the net it always was:
`cutoff=` (Hz, 20 to 12000, default 300) the filter's floor; `res=` (0 to 100, default 72) its resonance; `env=` (0 to
100, default 45: 100 is 6 octaves) how far each strike opens it; `accent=` (0 to 100, default 80) how much an accent does;
`drive=` (0 to 100, default 0) how hard the oscillator drives the filter's saturation, grit and not loudness. `decay=` is
its decay knob and `glide=` its slide time. They are not reserved words (`section drive` is still a section); on a drum
part they are errors, and on a voice that is not the acid voice a warning (it plays as it would without them). Long note,
with its decisions: `docs/beatscript/ext-acid.md`. Example: `examples/beatscript/acid-303.beatscript`.

`glide=`, `decay=` and `pitch-drop=` on a drum part are errors. Marks work on drums and tonal parts alike, in groups
(`[x x]&` flams both), alternations and `deg( )`. The net carries them on each note's binding (`glide`, `decay`,
`pitchDrop`, `ratchet`, `flam`; written only when set), and the engine plays them in the browser, the app and renders
alike. Glide and pitch drop bend wavetable voices; a pluck or a sung voice strikes the note as written. Example:
`examples/beatscript/trap-expression.beatscript`.

### Arrange mutations

`arrange { }` makes the arrangement a path through the program's state: a **mutation** between sections rewrites the
**text** of a `let` pattern, and every later section reads the new text. The whole page: [Arrange mutations](https://beatscript.app/docs/arrange).

```text
arrange {
  intro build drop
  ~ acid_a rot 2, hoov_a up 3        # from here on
  wubs build2
  ~3 acid_* drift 0.15               # a glob, on every 3rd pass only
  peak
  @ hoov_b rev                       # the next section only
  @ control off                      # the control deck: none of its lines fire in the next section
  outro
} loop 4 then outro end
```

| op | does (to the pattern's steps: see **The steps** below the table) |
|---|---|
| `rot n` / `rev` / `swap` / `mirror` / `shuffle` | rotate `n` steps later / backwards / swap neighbours / the first half, then backwards / a seeded permutation |
| `thin p` / `fill p` / `euclid k` | each sounding step rests with chance `p` / each rest takes the step before it with chance `p` / the hits re-spread as `k` Euclidean hits |
| `half` / `double` / `stutter n` | the first half at half speed / the pattern twice in the bar / the first `1/n` of it `n` times |
| `ratchet p` / `swing amt` | a sounding step struck twice with chance `p`: doubled on the grid (`*2`) when its step is longer than a sixteenth, else ratcheted inside it (`x:2`, again `x:4`, at most `x:8`), so no hit snaps / every second step later by `amt` of a step (on the sixteenth grid) |
| `up n` / `down n` / `oct n` / `invert` / `drift p` | scale degrees of the `key` up / down / octaves / turned over about the first note / a step up or down with chance `p`. A Roman numeral moves by degrees too (in A minor `i` up 2 is `III`, the key's own triad there; `V7` up 1 is `VI7`) |
| `vel n` / `accent p` | accents on the `n` strongest steps (`vel -n` takes `n` off) / an accent with chance `p` |
| `morph b p` / `xfer b` / `mask b` | each step becomes `b`'s with chance `p` / `b`'s rhythm with these notes / rests where `b` rests |
| `grow` / `decay` | one rest filled / one sounding step taken out (seeded; they pile up pass on pass) |
| `voice n` / `spread` | each chord `[a3,c4,e4]` or `chord(am7)` inverted `n` times / its every second note an octave up |
| `jux op` | the pattern stays the left copy, `op` of it plays hard right (`jux rev`) |

**The steps.** A pattern written as one `[...]` (`let acid = [1 1 ~ 1 3 ~ 1 5 ~ ...]`, a bar of sixteenths) is its steps, not
one step: `rot`, `thin`, `half`, `decay`, `ratchet` and the rest work inside the brackets, and the brackets stay. A `<...>`
cycle of bars (`let riff = <[...] [...]>`, `let chords = <i i iv v>`) is worked a bar at a time, each bar as its own steps,
the dice running on from bar to bar: `rot 3` rotates every bar, `accent 0.3` accents steps in them, never the cycle as a whole.
A cross operator (`morph b p`, `xfer b`, `mask b`) reads `b` the same way, bar for bar when both are cycles. A pattern a
mutation takes down to one step (`~ p half` on `x ~`) is still a pattern.

Try [arrange-in-eight-lines](https://beatscript.app/examples/arrange-in-eight-lines.beatscript).

It is one section played eight times. Each play changes a pattern with `~` (the hats, then the arp, and so on, pass on pass)
or, with `@`, for one play only (the kick doubles, then the arp halves, and each is back the next play). Three comments in the
file say it in words.

- **Chords are pitched.** A `let` of `chord(am7)` steps, or of `<chord(am7) chord(g7)>`, takes `up`, `down`, `oct`, `invert`,
  `voice` and `spread` as `[a3,c4,e4]` does: the call is written out as the notes it plays (`chord(gm7)` is `[g3,a#3,d4,f4]`),
  in the octave of the part that reads the let (`harmony` 3, bass 2, the rest 4, or its `oct=`), and a chord with an octave
  of its own (`chord(gm7, 4)`) keeps it. If parts in different octaves read the let, the first one's octave is used and
  `check` says so. An operator that moves no pitch (`rot`, `thin`) leaves `chord(am7)` as written.
- **Numerals and generators are pitched.** A `let` of Roman numerals (`<i i iv v>`) moves by scale degrees (above). A `let` of
  `markov(...)` or `arp(...)` is pitched as well: `oct n` plays it `n` octaves on (`octave(markov(0.3, 1303), 1)`), and
  `up n` / `down n` move the notes it picks by `n` degrees of the key (`markov(0.3, 1303, shift=2)`): the same tune, in the
  key, never a semitone transpose out of it (`down 2` after `up 2` is the text as written; the seed a part takes from its
  text leaves `shift=` out, so an unseeded `markov` keeps its tune). Its notes are picked as it
  plays, so `invert`, `drift`, `voice` and `spread` have none to move: `check` says so and leaves it as it is (write its
  notes out to move them).
- **`~`** rebinds the pattern for every later section, on every deck that reads it. **`@`** is on top of the current
  generation for the next section only. Ops in a chain (`rot 1 | thin 0.1`) apply in order; targets are comma-separated.
- **`~3` / `@4`**: a whole number right after the mark (1 to 256, spaced or glued, `~1` is no count) fires the line only on every
  Nth pass of the block: occurrences N, 2N, 3N ... counted from 1, so `~3` in `loop 6` fires on passes 3 and 6 and never on
  the first. The count is for the whole line (every target and chain); a glob's patterns fire together. Generations are
  numbered by firings (`acid@2` is the second time `~3` fired), and a one-shot `@4` that fires at the end of a pass is
  read by the next pass's first section. It counts passes of the block, not `evolve every N bars`, which counts bars inside
  a section. A line that can never fire (no `loop`, a `loop` with fewer passes than the count, a `loop` with no count,
  a named block) is a warning at its count.
- **The ending**: nothing or `end` plays the block once; `loop` plays on for ever, each pass a new generation (a render
  lays out one pass); `loop N then end` plays N passes, mutations piling up pass on pass, and every one is rendered;
  `loop N then outro end` adds a coda that reads the last generation.
- **`name@n`** in a section reads generation `n` of a pattern (`acid_a@0` is the text as written).
- **The control deck, per pass**: `~ control off` holds back the `control:` lines of the sections after it, `~ control on` lets
  them fire again, `@ control off` holds them back in the next section only, and `~ control freeze off` switches the lines of one
  call (the line's first word, `move` `swap` `crossfader` `tempo` `duck` `mix` ..., or its registry noun); counts work as for a
  mutation (`~2 control off`). A switch is a line of its own. It reaches a section's own `control:` lines (with its repeats and
  the defs it calls), not a line outside every section nor an engine call with no head; a section played with lines held back is
  a copy without them (`drop@2`), the held-back lines still checked. `check` says what each play holds back
  (`arrange.plays[].control_off`), and so does `listen` (`sections[].control_off`). See docs/beatscript/ext-control.md "Per pass: the
  switch" and `examples/beatscript/control-per-pass.beatscript`.
- **Deterministic**: the dice are a hash of (the program's CID, the pass, the mutation, the target, the operator); no clock.
  The same text is the same generations, so a track stays CID-reproducible. `check` reports the table (`arrange`: each
  play, its pass, and the generation and text of each pattern it reads; with a counted line, `fires`: the passes it fired
  on and the plays that first read each).
- `check`: an unknown pattern or operator is an error (the fixer comments a bad operator out as `id`); a glob that
  matches nothing and a pitch operator on a drum pattern (no pitch in it, or read only by drum parts) are warnings (the
  operator does nothing there). `arrange a b c` on one line is unchanged.
- **Stereo**: `jux op` keeps the pattern as the left copy and plays `op` of it as the right one (Tidal's `jux rev`):
  every part that reads it plays both, one hard left and its twin hard right, each at the level the part had. What comes
  before `jux` in a chain works on both copies, what comes after on the right copy only; a chain has one `jux`. A sampled
  voice plays in the middle, and a kick or bass leaves it: `check` warns. See [Arrange mutations](https://beatscript.app/docs/arrange).

### Start from a seed

`seed(genre, n)` is a track of the beats generator (the beats sequencer's, the same notes), as BeatScript: a value, never
audio. Start a program from one line, write it out, and make it yours:

```text
arrange from seed(techno, 42)                          the whole track
section intro from seed(techno, 42)[1..8]              bars 1 to 8 as a section
deck B: bass seed(techno, 42).bass[17..24] inst=fm-bass    a part's bars; your params win
let riff = seed(techno, 42).bass[17..20]               the same, named (say which part)
arrange from seed@2(jazz, 7)                           seeds v2: the track, swung
section hook from <cid>.drop                  a saved program's section (parts, key)
deck B: bass <cid>.drop inst=hoover           its part (the line's role; .melody another)
arrange from template(deep-house, @tempest_such_stuff)  a phrase as a track
arrange from template(rondo, @hands_up, refrain=ode_to_joy)  a form, a role bound: docs/forms.md
```

- The genre is one of the BeatScript ontology's (ambient blues bossa country dnb dubstep edm funk garage house jazz lofi metal
  reggae speedcore synthwave techno trance trap), `n` a whole number 0 to 4294967295, and a third argument a structure:
  standard (the default), loop, minimal or extended (`seed(house, 7, loop)[1..2]`). `[a..b]` picks bars (from 1, both
  in; `[9]` one bar), `.kick` `.bass` `.melody` ... a part, and they combine: `seed(techno, 42).bass[1..8]`. A part line's
  reference with no `.part` takes the line's role.
- It is written out before the program is lowered, so the browser, the phone and the booth all play plain BeatScript:
  every note on the grid where the generator plays it, the drums and the bass on deck A, the melody, harmony and arp on
  B, each part with its instrument (or kit), a `vel=` range with `!` on its accented hits and its commonest `len=`
  (so the velocities and lengths are near the generator's, not each one exact). `arrange from` adds the `tempo` and `key`
  a program does not say itself, and a `loop` track goes round. A bare `seed(...)` never adds a `swing`: seeds v1 write a
  swung genre's track straight. `seed@2(...)` adds the `swing` too, as the next bullet says.
- **Write it out to edit it**: `beatscript fmt --expand` (or `beatscript expand`, the MCP `expand` tool) gives the text a seed
  stands for, each place under a `# from seed(...)` line; `--lineage` opens it with `#! forked-from: <CID>` of the
  program as given, so a shared fork says where it came from. The written-out program plays the same as the reference.
  In the editor (beatscriptls), any reference hovers with its expansion, shows or hides it inline, and ejects in place: docs/beatscriptls.md.
- **The same music, always**: seeds are versioned, and the version is in the text, so in the CID. A bare `seed(...)` is
  seeds v1 for good (`seed@1(...)` is the same and prints bare; a `#! seeds: v1` line says so, and is what a program with no
  such line means). A seed always means the same track, so the CID of a program with references (of its text, references
  and all) never moves; a change to the generator that changed any seed is a new version, written in the reference.
- **Seeds v2, `seed@2(genre, n)`**: the v1 track with its genre's swing written. The generator's tables give each genre a
  swing from 0 to 100 (jazz 60, blues 50, bossa 40, lofi 35, funk and garage 30, reggae 25, house 20, dubstep and country
  15, dnb and trap 10, the rest 0), and v2 writes it as the program's `swing`, the value / 100 held to 0.6, as the
  seeded generator (`gen@N`) prints it: `arrange from seed@2(jazz, 7)` says `swing 0.6`, a straight genre says none, so
  `seed@2(techno, 42)` writes out the same program as `seed(techno, 42)`. Only `arrange from` writes the swing (it is a
  setting of the whole program, and a section, a part or a `let` brings none: `check` says so there, with the `swing` line
  that would), and the program's own `swing` wins.
  `seed@2` stands wherever `seed` does, with the same parts, bars and structures. A `#! seeds: v2` line is refused: a
  comment is outside the CID and pins nothing, so the version goes in the reference. Long note: `docs/beatscript/ext-ffi.md`,
  "`seed@2`".
- `check`: an unknown genre is an error with the nearest names; bars outside the track an error; a part the genre does
  not have a warning (it is silent); a version this BeatScript has not got (`seed@3(...)`) an error naming the ones it has; a
  head apart from its `(` (`seed@2 (jazz, 7)`) an error with the reference it would be.
  A structure is `seed(house, 7, loop)`: standard (the default), loop, minimal or extended.

### Borrow from a stored program

`<cid>.drop` is a section of a program that is already saved, and `<cid>.drop.bass` one of its parts: the music itself, named by
its hash, not a copy that can drift. It stands where a seed does (`ref` in the grammar): keep the drop of a track you like, and put your
own around it.

```text
section hook from z4EBG9j9....drop              a section of it, as a section of yours
deck B: bass z4EBG9j9....drop inst=hoover       its bass on this line; your params win
deck B: bass z4EBG9j9....drop.melody            another of its parts, on this line's role
let riff = z4EBG9j9....drop.bass                the same, named (a let says which part)
deck A: kick z4EBG9jF...                        a bare CID is a program with no sections: its kick
```

- A CID is `z` and 20 to 80 base58 characters, and is a word of the language: no `let`, `def` or section is named one. After it,
  `.section` and `.role` (a role is one of the eight; a section comes first): `<cid>.drop`, `<cid>.drop.bass`, `<cid>.bass`.
  A section line takes a section (a `.role` keeps just that part); a part line takes the line's own role unless `.role` says
  another; a `let` needs the role; a bare CID is for a program with no sections.
- **What comes along.** A section brings its parts as they play in its first stretch (each part as the canonical text says it, its lets
  and defs already worked out), its length in bars and, when it sounds in another key than yours, its `key`. It never brings
  its tempo, and never its **control lines**: filters, reverb, `move`, `mix`, `duck`, `sing`, `at bar` lines and `evolve`
  name sections and times that do not travel, so they stay behind and `check` warns, naming the kinds (`master: filter,
  master: reverb`). Write the ones you want in your own section. A part brings its pattern and its params (`kit`, `vel`, `inst`,
  `oct`, `len`, ...); the line's own params win. A part with chance brings the seed its dice come from, so it plays the same hits
  on any deck (a `vel=a..b` range is drawn from the deck too, so on another deck the same hits have other velocities). A `let`
  is the pattern alone, read in the key it lands in.
- **The language never fetches.** The source is found in what the host gives `check`: the programs a share link carries, then the
  host's saved programs (and the links it opened), the programs beatscript.app keeps, the programs `beatscript check --with` was given,
  the page's fetch by CID ("Share links", the `import` bullet). Each is hashed against its CID before it is used. One no
  host has is an error at the reference (`z...ukZ is not here: this program draws on it, and check needs its text`), the same
  words on every host: hosts differ in whether it is there, never in what the music is. The source is compiled in a plain
  environment (no registry, no instrument catalogue), so the slice is the same on every host; whether it *checks* is up to yours:
  an `inst=` your host lacks is an error on the reference.
- **The CID of your program is of your text**, with the full CID of the source in it: it never depends on whether the source
  can be found, a cycle is impossible (a text cannot hold its own hash), and borrows nest four deep. `check` lists what a program
  drew on (`uses`), a share link carries the source (and what it borrows from), the rights flow as for an `import` (a CC BY source is
  credited, an all-rights-reserved one warned of, a private saved one is not shared), and the edge is in the text a CID covers,
  so unlike `forked-from` it cannot lie.
- **Write it out to edit it**: `beatscript expand` gives the text a reference stands for, each place under a `# from <cid>.drop` line,
  and the written-out program plays the same. A section's `key`, a part's `seed=` and its params are written in.
- `check`: a section the source does not have is an error with the nearest names and the ones it has, a part it does not play an
  error naming the ones it does, a role on two decks an error (borrow the section), a section written and never played an error;
  a section with control lines left behind is a warning.

### Call the generator: `gen@v`

`gen@2 techno seed=42` is the program the seeded generator prints for those inputs: the generator of the MCP `generate` tool,
`POST /api/generate` and beatscript.app's ✦generate button (no model, no sign-in; the G key opens it). It is an engine call
("Two halves"): opaque until it is resolved, then pinned by the CID of the program it resolves to. Its params are the
generator's own, so a call reads like a function's signature:

```text
let bed = gen@2 techno seed=42 section=drop part=bass      the bass of its drop, a let a bar: bed_1 ... let bed = <bed_1 ...>
deck B: arp gen@2 house seed=7 section=drop inst=pluck      a part of it, on this line's role; your params win
section hook from gen@2 techno seed=42 section=drop         a section of it: its parts, never its control lines
arrange from gen@2 techno seed=42 bpm=128 key=am            the whole program
```

- **Params**: the genre, then `seed=` (a whole number or a word; a call always says its seed), `bpm=` (40 to 300), `key=`,
  `length=` (short, standard or long), and which piece: `section=` (one of the program's sections) and `part=` (a role).
  `seed=42` and `seed = 42` are one call.
- **The version is in the text**: `gen@1` is `seeded@1`, `gen@2` is `seeded@2`, `gen@3` is `seeded@3` and `gen@4` is `seeded@4` (a generated
  program's `#! generator:` line). A bare `gen` is `gen@2`, and the canonical text writes the version, so it is in the CID (a
  `#! seeds:` line is a comment, and is not). Every version stays callable: a generator that would print another program for the
  same inputs is a new version, and the old one is kept as it was. `gen@3` is `gen@2`'s program in BeatScript 2 (its version
  line first, "Versions"); `gen@4` is `gen@3`'s program with the acid genre's bass on the 303 (`inst=acid-bass`, where 3 had
  no acid voice to pick and wrote `sub-bass`), every other genre's the same but its `#! generator:` line; `gen@1` and `gen@2`
  still print BeatScript 1, byte for byte. The tests pin what a sample of calls
  resolves to.
- **It fits the program**: unless the call says its own `bpm=` or `key=`, it is generated at the program's `tempo` and in its
  `key`, so two seeds crossed with `morph` or `xfer` never clash. `check` says so at the call, with the CID: `gen@2 techno seed=42
  section=drop part=bass -> z4EB... (seeded@2): generated in the program's key am and at its tempo 124`. The fitting is part of
  the resolution: the same program text is the same call, the same generated program and the same CID.
- **What it is written out as**: a part line and `section X from` borrow from the generated program as from a stored one
  ("Borrow from a stored program": its parts and their params, never its control lines). A `let` is the notes the part plays
  there, a let a bar (`bed_1`, `bed_2` ... and `let bed = <bed_1 bed_2>`), so `~ bed_* rot 2` mutates it as it would a local
  let. `arrange from` is the whole program, less the `tempo`, `key` and `swing` lines the program says itself.
- **Swing**: a genre that swings in the generator's tables (jazz, blues, bossa, lofi, funk, garage, reggae, house ...) prints the
  program-wide `swing`, the table's 0..100 as 0..1 (at most 0.6: `gen@2 jazz` says `swing 0.6`, `gen@2 house` `swing 0.2`); a
  straight genre (techno, trance) prints none. Every version does. `arrange from` keeps it; a part, a section and a `let` never
  bring it, as they never bring the tempo.
- **Eject**: `beatscript expand`, the MCP `expand` and the editor write one reference out in place, under a `# from: <call> -> <cid>`
  line. The line is a comment: the ejected program has the CID of the same notes typed by hand, and the line keeps where they
  came from. Saved on the site, the program is stored as its canonical text with its `# from:` lines kept beside it, in
  `meta.based_on` (`["<call> -> <cid>"]`, each line once, in order; a `# from:` line with no CID is a plain comment, and expand's
  `# from <cid>.drop` over a borrowed section counts as `<cid>.drop -> <cid>`): its JSON-LD lists each in `isBasedOn` as
  `vendored`, and lineage counts it in `vendoredFrom`.
- **No fetch**: the generator is compiled into the host (`beatscript`, beatscript.app, the page's wasm, the editor). A host without it
  says `gen@2 is not here` at the call. `seed(techno, 42)` is another generator (the beats sequencer's, seeds v1) and stays as it was.
- `check`: an unknown genre or section is an error with the nearest names; a `let` names one section and one part; a `bpm=`,
  `key=` or `length=` it does not take is an error at the param. Long note: `docs/beatscript/ext-ffi.md`.

### Phrase templates

`template(name, phrase)` compiles a phrase, a phrasebook id or any words, to a whole track. Like a seed it is written
out as plain BeatScript before the program is lowered, so the track is text you can read, edit and re-check:

```text
arrange from template(deep-house, @tempest_such_stuff)      a phrasebook line: its words, its stress, its feel
arrange from template(deep-house, "robots on the road")      any words: their stress is worked out
```

- **The phrase is the vocal line and nothing else**; the arrangement is the template's own, fixed. A template is a
  BeatScript text with slots, kept in the language's crate; applying it fills the slots (the lyric, the vocal line's lets
  `vox_a`, `vox_b` ..., the vocal part's cycle, the shaker, the length of a section) and writes the result out. The
  program's own `tempo`, `key` and `swing` win over the template's, and the vocal's notes are degrees of the key, so the
  track follows it. The same template and phrase are the same text and CID; `template` is not a reserved word.
- **Syllables set the line's length**: one syllable to an eighth note, so a line is 1, 2 or 4 bars (a triplet line up to 8
  cycles of twelve steps), ending at least a beat before its cycle does, to breathe. A section is 8 bars (12 in the triplet
  feel), so the line is sung again in it; a phrase too long for a line (about 30 syllables, 45 in the triplet feel) is an
  error that says so.
- **Stress sets the accents and the beats**: a stressed syllable is accented (`!`) and sits on a beat, the unstressed
  ones follow it on the eighths, a leading run of them is a pickup, and a rest is left where an odd run would put the
  next stress off the beat.
- **The meter sets the subdivision**: straight is the sixteenth grid (a bar of eight eighths); the triplet feel, for
  dactyls and anapests (three syllables to the beat), is a cycle of twelve sixteenths, `/12`: six eighths, a 6/8 bar
  against the 4/4 drums, the shaker in it too.
- **The notes**: stressed syllables on chord tones (degrees 1 3 5 8), unstressed on passing degrees (2 4 6 7), along an arch,
  a fall, a rise or a wave that a hash of the words picks; the last syllable rests on 1, 5 or 8.
- **The phrase's profile**: a phrasebook id (`@tempest_such_stuff`) has its words, stress and feel as the book curated
  them. Any other text has them worked out: the words are `sing`'s, a function word (an article, preposition,
  auxiliary, pronoun or conjunction) is unstressed, the first syllable of any other word is stressed, and the feel is
  triplet when the stresses fall three syllables apart (dactylic, anapestic, amphibrachic), else straight.
- `check` says what the phrase gave, in a note (`template deep-house: 9 syllables, anapestic, triplet feel, a vocal line
  of 3 bars in sections of 12 bars`), and every generation of the arrangement as for any `arrange { }`; a word the
  lexicon does not know is the usual warning. An unknown template says the known ones, an empty phrase says what to
  write, an unknown id the nearest ids; each error is at the reference.
- **Write it out to edit it**: `beatscript expand` (`beatscript fmt --expand`, the MCP `expand` tool) gives the program the reference
  stands for, under a `# from template(...)` line. The two play the same.
- **The same track, always**: templates are versioned (`#! templates: v1`, also what a program with no such line means).
  A change that changed any track would be templates v2.

**Forms:** six templates are classical forms (quodlibet, passacaglia, canon, rondo, fugue, theme and variations) and a seventh,
`cross`, is a quodlibet of two sources; each is a section shape and a mutation plan built from public-domain themes (the theme
library, beatscript.app/themes, CC0), the phrase sung in one section: `docs/beatscript/forms.md` (https://beatscript.app/docs/forms.md).
The MCP `genre_template` takes a `form` or a `theme`, and `list_examples` with `themes` lists the library.

| template | is | example |
|---|---|---|
| `deep-house` | deep house at 122 (Am F Dm G, swing 0.12): a four-on-the-floor kick (`kick4`) and clap, `drm_hat`, `drm_shk` and `drm_rim` drums, a bass on the roots, a pad and a kalimba on the vocal line. `night` buries the voice (lower, darker, ducked under the kick), `spot` puts it up front. The arrangement: night, `@ vox_* oct 1` spot (an octave up), `~ drm_* fill 0.15`, night, `@ vox_* xfer drm_shk` spot (the voice crossed onto the shaker), `~ drm_* grow`, four passes | `examples/beatscript/such-stuff-as-dreams-122.beatscript` |
| `quodlibet` | a form: Pachelbel's ground and violin, Beethoven's Ode to Joy and fate motif take turns by section and cross-morph toward each other and away (`~ a_* morph b_1 p`, then `~ b_* morph a_1 p`), the phrase sung over the ground | `examples/beatscript/canon-of-fate.beatscript` (the reference) |
| `passacaglia` | a form: Holst's five-beat Mars ostinato as a ground never changed, cumulative variations above it (`rot`, `thin`, `rev | drift`, `grow`, `up`, `down`, `decay`) | `examples/beatscript/full-fathom-five.beatscript` (the reference) |
| `canon` | a form: Pachelbel's violin line chased by copies of itself a bar and a beat behind, made of its own bars | |
| `rondo` | a form: Joplin's Entertainer as a refrain returning mutated (A B A C A) between Bach's Prelude in C and the sung section | `examples/beatscript/rag-rondo.beatscript` (the reference) |
| `fugue` | a form: Grieg's Mountain King as the subject, answered a fifth up and re-entering transposed while Bach's Badinerie morphs toward it | |
| `theme-and-variations` | a form: Dvorak's New World Largo, a different one-shot transform each section (`rev`, `invert`, `stutter 2`, `oct -1`, `half`), no cross-morphs | |
| `cross` | a form: two sources take turns (`a_leads`, `b_leads`), overlap (`both`) and morph toward each other and back in cycles (`~ a_melody_* morph b_melody_1 p`, `@ ... xfer`); unbound, Ode to Joy over Pachelbel's ground and the Badinerie in B minor | `examples/beatscript/cross-two-seeds.beatscript` |

**Roles: the form is the structure, sources fill it.** After the phrase, a form takes `role=source` bindings; a role nobody
binds keeps the form's own material, so a reference with none is the text it always was:

```text
arrange from template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass, upper=ode_to_joy)
arrange from template(cross, @psalm42_deep_calleth, a=gen@1 dubstep seed=54876, b=gen@1 house seed=7)
arrange from template(rondo, @hands_up, refrain=gen@1 dubstep seed=54876, episodes=template(passacaglia, ground=[g1 ~ d2 ~]))
```

| form | roles: what each is (default) |
|---|---|
| `quodlibet` | `a`, `b`: two tunes drawn toward each other (Pachelbel's violin, Ode to Joy); `ground`: the bass (Pachelbel's ground); `third`: the line both are pulled toward (the fate motif in B minor) |
| `passacaglia` | `ground`: the bass under every section, never changed (Holst's Mars ostinato); `upper`: the line the variations change (the form's own) |
| `canon` | `subject`: the line chased by copies of itself, a bar and a beat behind (Pachelbel's violin); `ground`: the bass (Pachelbel's ground); the numbers `delay=` (beats behind) and `interval=` (semitones) of the chase's copy |
| `rondo` | `refrain`: the tune that comes back (Joplin's Entertainer); `episodes`: a line, the episodes' figure (Bach's Prelude in C), or sections |
| `fugue` | `subject`: entered, answered a fifth up, re-entered (Grieg's Mountain King); `counter`: drawn toward it (Bach's Badinerie) |
| `theme-and-variations` | `theme`: the one source each variation changes (Dvorak's Largo) |
| `cross` | `a`, `b`: two sources, each a melody, bass and chords (a whole seed; a line is the melody); one part alone: `a.bass=`, `b.harmony=` |

- **Sources**: a seed, `gen@1 dubstep seed=54876 [section=drop] [part=bass]` (the form fills in the role's part and the first
  section that plays it, the drop first); a public-domain theme by id (`ode_to_joy`); notes, a bar each (`[d2 ~ a1 ~]`,
  `<[d2 ~] [g1 ~]>`); a stored program's part (`<cid>.drop.bass`), and where a role takes sections a section of it
  (`<cid>.drop`) or the whole of it (`<cid>`: its sections in its order); a stored set (`<cid>`: its tracks in order, each its
  sections; `<cid>.track2` one track); another template, with no phrase (`template(passacaglia, ground=...)`): its sections
  under the role's prefix (`ep_ground`), its plan over the role's places; or one section of it (`template(cross).both`, and
  `.both.arp` its part).
- **Extensions** (docs/beatscript/forms.md "Extensions"): a section where a line goes gives the role its own part of it, as it first
  plays (`subject=template(cross).both.arp`: the cross's overlap as a canon's subject); a seed role's parts bind one at a time
  (`a.bass=[d2 ~ a1 ~]`, after the role in the canonical text); the canon's `delay=6` and `interval=7` put the chase's copy a bar
  and two beats behind, a fifth up (without them: a bar behind, an octave down); the quodlibet's roles. Each has a default that
  keeps every reference that does not say it written out as it was.
- **Written out as structure**: a role is a let a bar the sections and the plan read by name (`ostinato`, `subject`, `tune`,
  `dux`, `air`, `a_melody` ...: `~ subject_* rot 2`), or sections; a seed stays a `let NAME = gen@1 ...` line for the gen
  stage, pinned. Every section keeps a doc comment; the header says what fills each role.
- **Key**: a source plays in the key of the sections it fills (the program's, a seed fitted to it, a theme moved into it; an
  inner form's sections open with its own `key`, its seeds say it). The roles go in the form's order in the canonical text.
- **Total**: forms nest 3 deep, a line is at most 16 bars, a role of sections (a set's tracks too) at most 128 bars a pass; each
  an error at it.
- Eject writes the form out under `# from: <the reference> -> <CID>`, the whole chain of forms and seeds; the seeds in it eject
  in turn. Long note: `docs/beatscript/ext-roles.md`.

Three phrases, three tracks (`examples/beatscript/`): `such-stuff-as-dreams-122.beatscript`, Shakespeare's "we are such stuff as dreams are
made on", iambic, 20 syllables, a four-bar line in the bar; `assyrian-122.beatscript`, Byron's anapestic "The Assyrian came down like
the wolf on the fold", 12 syllables, the 12-step cycle; `commit-message-122.beatscript`, "make the drop deeper and move the
kick", a commit message whose stress is worked out. Long note: `docs/beatscript/ext-template.md`.

### Name a source: `use`

`use SOURCE as NAME` gives a source a name, on a line of its own at the top of the program, and the name is then used where a
section, a `let` or a phrase of the program's own would be. A long reference reads in lines, and a source used twice is named
once:

```text
use z4EBG9j9....drop as hook                             a stored program's section: arrange intro hook
use z4EBG9j9....drop.bass as riff                        one of its parts, a line: deck A: bass riff inst=sub-bass
use gen@2 house seed=7 section=drop part=bass as walk    a seed's part, a let a bar: ~ walk_* rot 2
use template(passacaglia).ground as ground               a canned section: a section of a template, written out
use template(deep-house, @tempest_such_stuff).night as night    the same, sung with that phrase
use genre@1 techno.drop as drop                          a genre template's section (genre_template's starter, frozen)
use theme ode_to_joy as tune                             a theme, in the program's key: deck B: melody tune inst=piano
use phrase @tyger_burning_bright as tyger                a phrasebook line: deck B: sing tyger on melody
use template(passacaglia, ground=walk) as episode_form   a form, for a role: template(rondo, @hands_up, episodes=episode_form)
```

| source | the name is | written out (first, before templates, seeds and calls) |
|---|---|---|
| `<cid>.drop`, `gen@2 ... section=drop` | a section | `section NAME from <cid>.drop`, which the stored stage writes out ("Borrow from a stored program") |
| `<cid>.drop.bass`, `<cid>.bass`, `gen@2 ... part=bass` | a line | `let NAME = <cid>.drop.bass` (its pattern), `let NAME = gen@2 ...` (a let a bar) |
| `template(passacaglia).ground`, `template(deep-house, @id).night` | a section | the template written out (with the form's own example phrase when it names none), then that section borrowed by its CID; a section whose parts read the form's own lets in a call (the canon's `chase`) is refused, and `list_examples` with `imports` lists the ones that can be borrowed |
| `genre@1 techno.drop` | a section | the genre's starting program as `genre_template` writes it, frozen in genre templates v1, then that section borrowed by its CID |
| `theme ode_to_joy` | a line | its bars as lets named for the name, `NAME_1` ... `NAME = <NAME_1 ...>`, moved into the program's `key` (to its relative key when the theme's mode is the other one), as a role moves one |
| `phrase @tyger_burning_bright` | a phrase | nothing: `sing NAME on ROLE` and `template(T, NAME)` sing `@id` |
| `<cid>`, `gen@2 ...` (no section or part), `template(passacaglia, ground=...)` | a source for a role | nothing: `ROLE=NAME` in a template reads the source |

- **In a role the name is its source.** `episodes=episode_form` is the form itself, nested as if written there; `subject=tune` is
  the theme, moved into the role's key by the role. A theme or a section whose name only roles read writes nothing at its line.
- **Pinned**: a CID names its text, `gen@N` its version, a canned section templates v1, a genre template `genre@N` (a bare
  `genre` is `genre@1`, and the canonical text writes it), a theme and a phrase their library (the file's CID). What is in the
  repo resolves offline, everywhere the language runs; a CID goes through the host's programs as any stored reference does.
- **The canonical text keeps the line** (`use theme ode_to_joy as tune`) and the name where it is used, so a program with a
  `use` is its own program, and no existing CID moves.
- **Rules**: a `use` is at the top; a name is given once and is not a `let`, section or def the program has (nor, for a line,
  one of its bars `NAME_1` ...); it is used after its `use` only, so no `use` reaches itself; it goes only where its kind goes
  (a section where `arrange` places one, a line where a part or a mutation reads one, a phrase where `sing` or a template's
  phrase is, a source after a role's `=`). Each is an error at the place, with the line that would work. A name never used is a
  warning.
- `check` says what each name is, in a note at its line (`` `tune` is a line: ode_to_joy (Symphony No. 9, Beethoven, 1824), 4
  bars of 16 steps, moved -2 semitones from d into am; play it as `melody tune` ``); `refs` lists the line (kind `use`), and
  `imports` each name with its source; `listen` marks the parts a `use` made with `from`, and its `name`. Eject (`beatscript expand`,
  the MCP `expand`, the editor) writes the line out under `# from: <source> -> <CID>`, and a name in a role or a phrase's place
  as its source. A program's JSON-LD and lineage say it is based on what it uses (`isBasedOn`: a stored program's CID, the
  call, the library). Long note: `docs/beatscript/ext-roles.md`, "Name a source".

## The language extensions

Fifteen features were added to the base language. Each section below says what the feature is, shows a few lines of a real
program from `examples/beatscript/` (every file there is original, CC0, and is compiled, canonicalised and reformatted at random by
`crates/djlive/tests/examples_beatscript.rs`), gives the rules in a few bullets and the canonical form, and links the long note.
The long notes hold the full grammar, the semantics, every decision and the errors; they are not repeated here.

| feature | example | long note |
|---|---|---|
| Arithmetic and guards | `examples/beatscript/arith.beatscript` | `docs/beatscript/ext-arith.md` |
| Time finer than a bar | `examples/beatscript/timing.beatscript` | `docs/beatscript/ext-timing.md` |
| Tempo curves | `examples/beatscript/tempo.beatscript` | `docs/beatscript/ext-tempo.md` |
| Shapes and lfo | `examples/beatscript/shapes.beatscript` | `docs/beatscript/ext-shape.md` |
| Degrees, numerals, a key a section | `examples/beatscript/scale.beatscript` | `docs/beatscript/ext-scale.md` |
| Euclidean rhythms and polymeter | `examples/beatscript/euclid.beatscript` | `docs/beatscript/ext-meter.md` |
| Seeded chance | `examples/beatscript/chance.beatscript` | `docs/beatscript/ext-chance.md` |
| Composition: stack and then | `examples/beatscript/algebra.beatscript` | `docs/beatscript/ext-algebra.md` |
| Typed ports | `examples/beatscript/ports.beatscript`, `ports-import.beatscript` | `docs/beatscript/ext-ports.md` |
| Mix moves: duck and mix | `examples/beatscript/mix.beatscript` | `docs/beatscript/ext-mix.md` |
| Sung lyrics | `examples/beatscript/sing.beatscript` | `docs/beatscript/ext-sing.md` |
| Phonemes, the phrasebook, the vocoder | `examples/beatscript/vocals.beatscript` | `docs/beatscript/phonemes.md` |
| Assertions and `btw test` | `examples/beatscript/assert.beatscript`, `examples/beatscript/constraints.beatscript` | `docs/beatscript/ext-assert.md` |
| Performed inputs | `examples/beatscript/perform.beatscript` | `docs/beatscript/ext-perform.md` |
| Links to the ontology | `examples/beatscript/linked.beatscript` | `docs/beatscript/ext-linked.md` |
| Pattern sugar | `examples/beatscript/sugar.beatscript` | `docs/beatscript/ext-sugar.md` |
| Call and response | `examples/beatscript/answer.beatscript` | `docs/beatscript/ext-answer.md` |

### Arithmetic and guards

Every place the language takes a number takes arithmetic: `+ - * / %`, parentheses, a unary minus, `min(...)` and
`max(...)`. It is worked out **once, when the program is read**, over exact rationals, so a program stays total and
deterministic: there is no loop, no state and no clock in an expression. `$i`, `$n` and a def's `$params` are put in place
as tokens before an expression is read, so `$i * 2 + 1` is `3 * 2 + 1`.

```text
#! beatscript 2
tempo 120 + 4 * 2            # 128
key am
swing 1 / 16 + 1 / 16        # 0.125

let total = 4 * 4            # a `let` of arithmetic is one number, put in place of every later `total`
let base = 60 + 12           # 72

def pulse(deck, step) {
  deck $deck: hihat x*(4 + $step % 2 * 4) vel=(base + $step * 6)..100 when $deck == A
}

section groove total bars {
  deck A: kick e(2 + 2, 16) | bass 1 ~ (1 + 2) 5 oct=3 - 1
  deck B: harmony chord(am7)
  pulse(A, 1 + 1)
  pulse(B, 3)                # dropped: its guard is false for deck B

  repeat 4 {
    at bar $i * 4 + 1: master: filter lp 300 * ($i + 1) -> 2k over 2 bars when $i % 2 == 0
    at bar $i * 4 + 3: master: reverb min(0.5, ($i + 1) / 8)
  }
}
```

- **Exact.** `/` is exact (`1 / 4` is a quarter), `%` is Euclidean (`-1 % 4` is 3), and an overflow or a division by zero
  is an error at the operand, never a wrap. A number that must be whole (a bar, a count, a degree) says so when the result
  is not (`repeat 3 / 2`). Decimals are read as the rational they were typed as (`0.3` is 3/10, at most six places).
- **A lone literal is not arithmetic.** `2k`, `50%` and `0.123456789` keep their spelling and their exact value;
  arithmetic starts at an operator, a `(`, a `$name` or `min(`. Where numbers stand side by side (an `evolve`'s
  `transpose <0 -5 7>`) the literal reader is kept.
- **`let` folds**: `let n = 2 * 4` is the number 8, put in place of every later `n`. `let r = 3*2` is 6 (it once was the
  degree 3 played twice: write `let r = [3 3]`), the only change of old meaning.
- **Slots glued to what comes before them take arithmetic in parentheses:** `hihat/(4 + 8)`, `x?(1 / 4)`, `x*($i % 3 + 1)`,
  `weights (1 + 1) 1 1`. A def's arithmetic argument is put in place in parentheses so that its grouping survives, where
  the slot it lands in needs them (beside an operator, glued to what comes before, a step that was never a pattern); an
  argument of stars in a step of a pattern (`1*4`, `1*2 3`) is left as written, as it always meant.
- **Guards** (`when`, above) are the other half of this feature: a condition over the same arithmetic and words.

Canonical form: a lone literal keeps today's spelling; arithmetic that holds nothing read while the program plays is the number
it comes to, wherever a number goes: `tempo 60 + 6` is `tempo 66`, `level 0.25 + 0.25` is `level 0.5`, `e(1 + 2, 8)` is `e(3, 8)`,
`x*(1 + 1)` is `x*2`, `section drop 4 * 4 bars` is `section drop 16 bars`; one program each. A rational prints as its decimal when
it has one of at most six places (`0.7`) and otherwise as the exact division (`1 / 3`), so what the text says is exactly the
number. An expression with an input in it (`knob(3)`, `held(key)`) stays an expression, with every constant piece folded
(`level knob(1) * 0.7`, `when knob(1) > 0.5`: one space round a binary operator, parentheses only where precedence needs them). A
scalar `let` prints as its value (`let third = 1 / 3`); a guard prints after its line. A mix item is exactly a decimal of at most
six places or an error. Long note: `docs/beatscript/ext-arith.md`.

### Time finer than a bar

`over 1/2 bar`, `over 2 beats`, `for 2 beats`, `every 2 beats`, `every 1/2 bar from 3`, `at bar 16 beat 3`,
`before drop 2 beats`. A beat is a quarter of a bar.

```text
section intro 4 bars {
  deck A: kick x ~ x ~ | hihat x*8 vel=50..80
  deck B: harmony chord(dm7)
  at bar 1 beat 2: master: filter lp 400 -> 3k over 2 beats     # a ramp half a bar long: it ends two beats after it starts
  at bar 2 for 3 beats: deck B: mute                              # muted for three beats, then put back
  before drop 1/2 bar: master: reverb 0.1 -> 0.7 over 2 beats   # the swell into the drop, a half bar ahead of it
}

section drop 8 bars {
  deck A: kick x x x x | snare ~ x ~ x | hihat x*8 vel=60..100
  deck B: bass d2 ~ d2 ~
  every 1/2 bar from 1: deck B: level 0.8                        # a pulse every half bar from the drop's second bar
  at bar 7 beat 5/2: master: delay 0.3                           # beat 2 and a half of bar 7
}
```

- **Beats count from 0, like bars.** `at bar 16 beat 3` is the last quarter of the bar whose line is bar 16; `beat 0` is the
  line itself; a beat is `0 <= b < 4` and `beat 4` is an error that says to write `at bar 17`. A position is one exact
  number of bars (`at bar 3/2` is `at bar 1 beat 2`), and the bar and the beat are arithmetic.
- **The clock is the deck's beat position, not seconds.** A line with a time between bar lines fires when the position
  crosses that time, once; a clock that jumps (a stall, a seek) fires what is due in between, in order, at most 32 times for
  one repeating line. An `every` finer than 1/64 of a bar is an error ("faster than the clock").
- **A ramp over a fraction of a bar ends exactly**: the pass that reaches its end sends the end value once. A hold of whole
  bars is put back on the bar line, as before; `for 2 beats` is put back two beats later.
- **Whole bars where the bar is the unit:** a section, an `evolve`'s generation, `repeat ... every`, a `move`'s `over` and a
  def's call (`at bar 2: groove(A)`) are whole bars. A position inside a section counts from the section's first bar.
- **A time past the end of its section is a warning:** `at bar 9` in a 4-bar section is due after the section, and `check` says
  "this line is due at bar 9 of section intro, which is only 4 bars long (bars 0 to 3)"; so is an `every` that first fires after
  it, and a ramp that is still moving when the section ends (it goes on after it: a ramp is not cut).

Canonical form: a duration prints `N bars` when `N` is whole (as it always did, `1 bars`), else `N beats` when the beats are
whole, else `P/Q bars` in lowest terms (`over 1/2 bar`, `over 2 beats` and `over 4/8 bars` are one program, `over 2 beats`); a
position prints `bar W` or `bar W beat B` (after `from`, a whole bar stays a bare number). Every whole-bar spelling prints
exactly as before, so no CID moved. Long note: `docs/beatscript/ext-timing.md`.

### Tempo curves

`tempo 100 -> 130 over 8 bars exp` ramps the tempo and holds the end; a `tempo` in a section sets it from the section's
first bar; `at bar 16: tempo 128 -> 140 over 4 bars` is a timed one.

```text
#! beatscript 2
tempo 100 -> 128 over 8 bars s-curve   # from the first bar: it speeds up through the intro, then holds 128
# ...
section build 8 bars {
  tempo 128 -> 140 over 6 bars exp                    # a tempo in a section runs from the section's first bar
# ...
}

section drop 16 bars {
  tempo 140                                           # a plain tempo holds until another one says
# ...
  at bar 12: tempo 140 -> 132 over 4 bars             # a timed line: it eases back before the end of the drop
}

section breakdown 8 bars {
  tempo 132 -> 90 over 8 bars log                     # fast at first, then it settles: the slowdown
# ...
}
```

- **Linear in beats, so the bar clock never moves.** A bar is four beats at any tempo, every time word means what it meant, and
  what a curve changes is the *seconds*, an integral worked out exactly (closed form per segment; the one numeric piece, the
  `s-curve`, is exact to a double). `Compiled::tempo_map()` gives `seconds_at(bar)` and `bar_at(seconds)`, and `check`'s
  `length:`, `assert length`, the offline render and the runtime all use it.
- **A tempo is an event, not a control.** It has one place (`at`, `after`, `before`, or none; `every` and `on phrase` are errors,
  `repeat N every M bars { tempo ... }` is the way to say it more than once), no target, and no `for` (end it with a later
  tempo). Two events at one place: the later wins. A ramp the next event cuts short warns. 40 to 300 BPM, everywhere.
- **The runtime sets the decks' tempo each service pass** from the map (the program's tempo is the master), sending only what
  moved. A program whose tempo is one number is not driven at all, so nothing written before moves. A host that cannot set a
  tempo is told once in `get`; the program goes on.
- **A program with no sections has its curve on the first bar you hear.** Its `at bar` lines count from the bar it was evaluated in
  (see "Time words"), but the tempo map starts on the bar the music lands on, the bar line after the evaluation when a deck was playing:
  `tempo 100 -> 140 over 4 bars` is at 100 when the net lands and at 140 four bars later, as the render has it (a song's curve starts on
  its own first bar, which is the same bar). Before the net lands, the curve's first tempo is already sent.
- **The nets of a section are made at the tempo the curve has where the section begins** (only for a program with a curve; a
  constant tempo keeps the nets and CIDs it had). Two stacked sections that set different tempos on one bar are an error.

Canonical form: `tempo 100 -> 130 over 8 bars exp` (`linear` is the default and is not printed); a plain `tempo N` prints as it
did; a timed line prints as a line (`at bar 16: tempo 128 -> 140 over 4 bars`). Long note: `docs/beatscript/ext-tempo.md`.

### Shapes and lfo

A ramp can go another way than straight: `master: filter lp 300 -> 8k over 8 bars exp`. A value can keep moving:
`master: filter lp lfo sine 2 bars center=3k depth=1k`.

```text
section intro 8 bars {
# ...
  master: filter lp 300 -> 8k over 8 bars exp         # the riser: it opens slowly and then fast
  master: reverb 0.1 -> 0.6 over 8 bars s-curve       # a swell that is gentle at both ends
  deck B: level 0 -> 1 over 4 bars step 4             # four leaps of a quarter of the ramp each
}

section drop 16 bars {
# ...
  master: filter lp lfo sine 2 bars center=3k depth=1k            # a wub that lasts as long as the drop: 2k to 4k
  master: delay lfo tri 1 bar center=0.3 depth=0.2 phase=0.25 over 8 bars   # a triangle, a quarter of the way round, for 8 bars
  deck A: level lfo saw 4 beats center=0.8 depth=0.1              # a sawtooth a bar: it rises, falls back and rises again
  do vocoder set mix=0 -> 1 over 8 bars log                       # shapes work in `do` lines too
}
```

| shape | `shape(t)`, t in 0..1 | at 1/4, 1/2, 3/4 | feel |
|---|---|---|---|
| `linear` | `t` | 1/4, 1/2, 3/4 | the default, never printed |
| `exp` | `(2^(4t) - 1) / 15` | 1/15, 1/5, 7/15 | slow start, fast end: a riser |
| `log` | `1 - (2^(4(1-t)) - 1) / 15` | 8/15, 4/5, 14/15 | fast start, slow end |
| `s-curve` | `3t^2 - 2t^3` | 5/32, 1/2, 27/32 | gentle at both ends |
| `step N` | `floor(t N) / (N - 1)`, at most 1 | N held values, N - 1 leaps | `2 <= N <= 1024` |

- **An lfo is `center + depth * wave(2 pi (t / period + phase))`**, in the field's own unit. `depth` defaults to 1, `center` to 0,
  `phase` to 0 (a share of one period). The waves `sine`, `tri`, `square`, `saw` all rise from the midline at phase 0. The
  period is a duration (`2 bars`, `4 beats`; at least a quarter of a beat).
- **How long:** `over dur` when it says, else until its section ends (in a stack, where its own section ends), else until the
  program stops. A filter's lfo swings between two knobs, linear in the filter's knob.
- **Out of range** (a swing past the param's range) is held at the edge, with a warning. **A depth left out is 1 in the target's own
  unit**: a whole swing of a 0..1 knob, but a hertz either side of a cutoff, which nobody hears: `check` warns when a filter's lfo has
  no `depth=` ("this lfo has no `depth=`, so it swings 1 either side of its centre: next to a low-pass cutoff in hertz that is no
  movement at all"). The default itself is the contract's and does not change (a default by target would move programs).
- **Everywhere a change takes a value:** the control lines (`filter`, `delay`, `reverb`, `crush`, `drive`, `resonance`, `level`, `low`,
  `mid`, `high`, `crossfader`) and `do` lines (`value=lfo tri 1 bar depth=0.2 center=0.3`, `mix=0 -> 1 over 8 bars log`). A
  tempo takes a shape but no lfo; a `mix` item takes neither (it is one number set once).

Canonical form: `lfo sine 2 bars center=3000 depth=1000 over 16 bars`: the wave, the period as a duration, the params sorted by
name with defaults left out, then `over`; a ramp's shape comes after its length. Long note: `docs/beatscript/ext-shape.md`.

### Degrees, numerals, a key a section

`deg(1 b3 5' ~)` writes degrees with accidentals and octave marks; Roman numerals (`i VI III VII`, `V7/7`) are chords of the key, as a
step or as the chord of `chord()` and `arp()`; a section may open with its own `key`; `voice-lead` moves chords as little as it can.

```text
#! beatscript 2
tempo 96
key am

let tune = deg(1 3 5 ~ #7 1' 5 1,)

deck A: kick x ~ x ~ | hihat x*8 vel=60..90
deck B: harmony i VI III VII voice-lead | bass i VI III VII | melody tune

section verse 8 bars {
  deck B: arp i ii° V7/7 i voice-lead
}

section lift 8 bars {
  key cm
  deck B: arp i bII V7 i
}

arrange verse lift verse
```

- **`deg( )`** is one slot whose terms are degrees of the key in force, counted in that key's own scale. `b3` and `#4` are a
  semitone down and up *from the scale's own degree* (in A minor `b3` is B); `5'` is an octave up, `1,` an octave down. A bare
  `b3` outside `deg( )` is still the note B3, and `#4` outside parentheses is a comment.
- **Numerals** are steps of `bass`, `melody`, `harmony` and `arp`. The case is the chord (`V` major, `v` minor), the quality is
  one of `7 maj7 ° dim °7 ø aug sus2 sus4 6 9`, `bVII` and `#iv` move the root a semitone, `I/3` `I/5` `V7/7` put that factor
  in the bass. `harmony` plays the chord, `bass` and `melody` its bass tone, `arp` the chord one note at a time over its share of
  the bar (four steps). A numeral in a drum part is an error. A numeral may name a `let`, a `def` or a section (the `let` wins in
  a pattern, so `let v = c2 e2` and then `bass v` plays the let; `V` is still the numeral), except a spelling whose canonical text
  is another word (`idim` is `i°`).
- **Numerals in calls**: where `chord()` and `arp()` take a chord symbol they take a numeral, read in the key in force: `arp(VI, up, 8)`,
  `chord(bVII, 3)`, `chord(V7/7)`. `chord(V7, 3)` plays the notes `harmony V7 oct=3` plays (the root above the tonic, as for a step), and
  a section's `key` moves it: `let arps = <arp(i, up, 8) arp(VI, up, 8)>` follows `key cm` as `<i VI>` does, where `arp(am, up, 8)` stays in
  A minor. `<I V vi IV>` is one chord a bar; bare, `I V vi IV` is one a quarter. `generate` and `genre_template` print numerals, so a
  `key` line moves everything they write.
- **A key a section**: `key cm` as the first statement of a section re-reads its degrees, numerals, `x` hits in tonal parts and
  the `markov` default (the lines the song keeps outside every section too). The program's `key` is the default and tags the
  share. Two sections that play at the same time in different keys are an error naming both.
- **`voice-lead`** voices each chord after the first, over every inversion and the octaves three below to three above, for the
  least total semitone motion from the chord before; ties go to the lower inversion, then the lower octave. Each part and
  section starts afresh.
- **A comment starts with `# `** (hash and a space): `#iv` is a sharp numeral, so a trailing `#I like it` is a parse error.

Canonical form: `deg(1 b3 5' 1,)` one space apart, marks glued; a numeral's quality in its canonical case (`VII°` is `vii°`,
`Idim7` is `i°7`, `iaug` is `Iaug`); `voice-lead` prints `voice-lead=on`; a section's `key` is its first line. Long note:
`docs/beatscript/ext-scale.md`.

### Euclidean rhythms and polymeter

`e(k, n, rot)` is checked and named; `role/N` gives a part a cycle of its own.

```text
#! beatscript 2
tempo 122
key dm

deck A: kick e(4,16) | snare e(2,16,4) | hihat/12 x x ~ x vel=55..85
deck B: bass/12 e(5,12) oct=2 | melody/10 d4 ~ f4 ~ a4 vel=70..95 | harmony/32 chord(dm) ~ ~ chord(bb) oct=3
```

- **`e(k, n[, rot])`**: `0 <= k <= n`, `1 <= n <= 64`, any whole rotation (taken modulo `n`). `check` names the rhythm when the
  BeatScript ontology names it for the part's own role (`e(3,8)` on a snare is "house snare"; `kick e(5,8)` is not named), to a
  program that is linked to the ontology (it has a `#! genre:` or `#! tags:` line: an old program that never asked is not told,
  there being no information level for a note). The ontology has no folk names, so none is invented.
- **`role/N`**: the part's cycle is `N` sixteenth steps (1 to 64) instead of 16, its terms sharing it as a bar's terms share the
  bar. `/` touches the role and the number. A cycle that is not the bar repeats against the barlines, so the part drifts: a
  polymeter, unrolled over `lcm(N, 16) / 16` bars (`/12` is 3 bars, `/10` is 5, `/32` is 2).
- **The period.** `check` says when it all comes round (`period: 30 bars = 480 steps (hihat/12, bass/12, melody/10 and
  harmony/32 against the bar and each other)`). A deck whose parts with a cycle of their own take more than 64 bars to come
  round is an error that names the parts and the way out (cycles that share a factor with each other and with 16, or two decks).

Canonical form: `hihat/12 x x ~ x`; `/16` is the bar and is not printed; `euclid(...)` prints `e(...)`; a rotation prints as typed
(`e(3, 8, 10)` and `e(3, 8, 2)` play the same notes and are two programs). Long note: `docs/beatscript/ext-meter.md`.

### Seeded chance

Parts that differ from bar to bar and are still **a function of the text and the bar**: `x?0.3`, `choose [a b c] weights 3 1 1 seed=4`,
`maybe(p, 0.3, 4)`.

```text
#! beatscript 2
tempo 128
key fm

section groove 16 bars {
  deck A: kick x ~ x ~ | hihat x*16?0.45 vel=45..85 | clap maybe([~ x ~ x], 0.8, 4)
  deck B: bass choose [f1 ab1 c2] weights 4 2 1 seed=4 ~ f1 ~ oct=1 | melody [c4 eb4 g4 ab4]?60% ~ c5?0.25 ~ inst=pwm-lead
}
arrange groove
```

- **A chance is decided by an integer hash of (seed, bar, cycle, step, note)**: `x?p` keeps the hit when `hash < floor(p * 2^32)`, so
  `p` is exact (`0.3` is 3/10, at most six decimals; `30%` is the same) and the same program is the same notes on every machine.
  The seed is the term's own, else the part's `seed=`, else a hash of the deck, the role and the pattern (not its `vel=`, `oct=` or
  `inst=`: nudging those does not re-roll which hits drop; a `markov` part with no chance still hashes its whole text).
- **`?` touches its term and its number** (`x*8?0.7`; a repeat, accent or slide comes before it). `x?0.5?0.5` must pass both.
- **`choose [a b c] weights 3 1 1 [seed=4]`** is one option a bar, picked by the same hash walked over the cumulative weights; there
  are exactly as many weights as options. `maybe(p, prob[, seed])` is `p` with `?prob` on each hit.
- **A hit that is not kept does not exist**: it is not in the net, so its token does not light. A chord is tossed note by note.
- **Unrolled over bars:** a pattern with chance is as long as `16` bars with no sections, the section's length (up to 64) in a section,
  rounded up to whole copies of its alternations and cycles. `check` reports every chance term (probability, seed, "differs
  every bar, unrolled over 16 bars"). A chance net has **no period**: `assert period` of a program with chance says so, whatever
  length it is unrolled over. `?0` is a rest and `?1` the bare term in the canonical text.

Canonical form: `x*8?0.7`, `choose [a b c] weights 3 1 1 seed=4`, `maybe([x ~ x ~], 0.3, 4)`; `?30%` prints `?0.3`; a seed prints only
when the term has its own. Long note: `docs/beatscript/ext-chance.md`.

### Composition: stack and then

A song is a composition of sections, and the two ways of composing them are in the language: `stack(a, b)` plays them at the same time,
`then(a, b)` one after the other, `silence` is the empty section, and a `let` names a composition.

```text
section groove 8 bars {
  deck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8 vel=60..90
}

section wash 8 bars {
  deck B: harmony <chord(am) chord(f) chord(c) chord(g)>
}

section riff 4 bars {
  deck B: melody <a4 c5 e5 g5> | bass <a1 ~ c2 ~>
}
# ...
let verse = stack(groove, wash)

arrange intro stack(then(riff, riff), verse) coda
end after coda
```

- **`stack(a, b)`** is as long as the longer; the shorter is padded with silence (its decks stop where it ends).
  **`then(a, b)`** plays `a` and then `b`. `arrange a b c` is `then(a, b, c)`. Over **parts** after `deck X:`, `stack`
  is `|` and `then` of parts of one role (same params, same cycle) is the per-bar alternation `<p q>`: `deck A:
  then(kick x ~ x ~, kick x x x x)` is `kick <[x ~ x ~] [x x x x]>`.
- **Laws, normalised when the text is read.** `then` and `stack` are associative (nested ones flatten), `silence` is dropped, a
  one-operand `stack` or `then` is its operand, `stack` is symmetric (operands sorted by canonical text), `then` is never reordered,
  and `stack` is not idempotent (`stack(a, a)` keeps both copies, which clash). Programs equal by the laws have one text and one CID.
- **Meaning.** The expression is evaluated to placements (a section at a start bar) and the song is cut into
  *stretches* where a placement starts or ends (`groove+riff`, then `groove`); every later stage reads those as
  ordinary segments. A later stretch goes on from where its section was (alternations, chance and hash velocities as
  if it played whole); a section's control lines fire once.
- **Clashes are errors:** two sections that play at the same time and both define one deck's role (naming both),
  different keys, different tempos on one bar, and a section with an `evolve` stacked with another. `stack` and `then`
  nest at most 32 deep.

Canonical form: the normal form of the laws; a plain `arrange a b c` prints as it always did; `let both = stack(verse, intro)` prints
`stack(intro, verse)`; names are opaque (`let both = stack(a, b)` is not the text `stack(a, b)`). Long note: `docs/beatscript/ext-algebra.md`;
the categorical reading is under "Categorical notes".

### Typed ports

A `def` may declare a **boundary**: the values it needs (`in:`) and the values it makes (`out:`), each a type. `wire changes.out ->
walker.chords` glues an output to an input, and the glue is checked.

```text
def changes() out: chords {
  emit <chord(am7) chord(fmaj7) chord(cmaj7) chord(g7)>
}

def home() out: key {
  emit am
}

def walker(oct) in: chords, key out: bass {
  deck B: harmony $chords | melody markov($key, 0.5) oct=$oct
  emit [a2 ~ a2 ~ f2 ~ g2 ~]
}

def thump() in: bass {
  deck A: kick x ~ x ~ | bass $in
}

wire changes.out -> walker.chords
wire home.out -> walker.key
wire walker.out -> thump.in

walker(4)
thump()
```

- **Types:** `chords key bass melody drums rhythm bars tempo pattern`. An input takes an output of the same type; a
  `pattern` input takes any pattern kind; a `rhythm` (hits and rests) fits `bass`, `melody` and `drums`. Nothing else:
  `type error: groove.out is a drums pattern, walker.chords wants chords`.
- **Wires join defs, not calls.** A call plays a def's lines; a wire carries values. A def reached only through a wire is never called
  and does not play: only its out values are made. `$chords` (or `$in`, when it is the only one) is the wired value; a pattern of more
  than one term goes in as one group.
- **Each input is wired once** (an output may feed many); **no loops**, through any number of defs (`the wires make a
  loop: alpha -> beta -> alpha`); an unwired input is an error at a call. An `emit` is fixed by the def's inputs,
  never by its parameters, so an out value is the same wherever the def is wired.
- **Bare chord symbols play:** `emit <am7 g7>` flows as `<chord(am7) chord(g7)>`. **Imports** bring a snippet's defs
  with their ports and its `wire` lines, so a saved composition keeps its wiring and the canonical text holds the
  snippet's CID.

Canonical form: `def walker(oct) in: chords, key out: bass {` (ports sorted by type); `wire changes.out ->
walker.chords` (a sole port is written `in` / `out`); `emit` lines print as body tokens. A def with no ports is the
macro it always was. Long note: `docs/beatscript/ext-ports.md`; the cospan reading is under "Categorical notes".

### Mix moves: duck and mix

```text
section drop 16 bars {
  deck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8 vel=60..90
  deck B: harmony chord(am7) inst=warm-pad | bass <a1 ~ c2 ~>
  duck B under A.kick 6db attack=5ms release=120ms
  mix {
    A level 0.85
    B level 0.7
    B eq low -6
    master level 0.9
  }
}

section break 8 bars {
  deck B: harmony chord(am7) inst=warm-pad
  mix { B level 0.9, master level 0.8 }
}
```

- **`duck B under A.kick 6db attack=5ms release=120ms`** lowers deck B by 6 dB while A's kick sounds. It is the
  engine's own sidechain, the `trick` kind `pump` (docs/TRICKS.md): a kick detector on A's pre-fader signal, so it
  adds nothing to the engine. `6db` becomes the pump's `amount` (`1 - 10^(-dB/20)`, 0.499), the release becomes the
  nearest note (120 ms at 124 BPM is a 1/16), and `attack=` is kept in the program (the pump's is 2 ms; a slower one
  warns). Only a low part can key it: `A.kick` (and `A.bass`, with a warning); a snare is an error with the way out.
  It lasts the section it is in (outside a section, until the program ends or `btw stop`).
- **`mix { ... }`** items are `A level 0.7` (the fader, 0 to 1), `A eq low -3` (a gain in dB, -24 to +6, the mixer's
  own scale: -3 dB is the knob 0.4375; `mid`, `high`), `master level 0.9`. Items are separated by commas or lines;
  each number is arithmetic, evaluated once. `A pan` is read and refused (the mixer has no pan).
- **In a section the mix is applied on its first bar and put back where the section ends.** The runtime reads the
  mixer first (`mixer {}`) and puts each knob back to what it held; with no read it goes back to the program's own
  last line, else to the neutral position, and says so. Outside a section a `mix` just sets; with `for N bars` it is
  put back after N.
- **A mix item is one number set once**: a ramp, an lfo or an input in one is an error that names the control line to
  write. Two mixes holding one knob at the same time are an error. The offline render has both.

Canonical form: a duck prints both times, defaults filled in, always in ms (`release=0.12s` is `release=120ms`),
params sorted; a `mix` prints one item a line with each number as the value it comes to. Long note:
`docs/beatscript/ext-mix.md`.

### Sung lyrics

```text
#! beatscript 2
deck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8 vel=60..90
deck B: bass a2 ~ e2 ~ | harmony chord(am) | melody <[a4 c5 e5 c5] [g4 b4 d5 b4]>

section verse 8 bars {
  # eight syllables on the melody's eight notes (it takes two bars to come round), sung over the whole section
  deck B: sing "lamps in the harbor, glowing slow" on melody voice=alto
}

section answer 4 bars {
  # a shorter lyric wraps over the two bars it is told to cover
  deck B: sing "hush _ lan-tern" on melody bars=2 voice=robot-choir
}
```

- **`sing "words" on melody [voice=NAME] [bars=N] [key=K]`** sets typed lyrics to the notes of one of the program's
  own parts (a tonal role; a drum has no pitch). Syllable *i* of the lyric is sung on note *i* of the part, timed to
  the part (the line has no rhythm of its own); a note is a part's onset (the highest of a chord) lasting to the next
  less a sixteenth of air. The lyric wraps while there are notes left and stops when they run out (a warning says how
  many).
- **The cover:** `bars=N`, else the section's length, else the part's own cycle (at most 32 bars).
- **Syllables:** vowel groups (`road` 1, `robots` 2); a hyphen cuts a word (`lan-tern`); `_` is a rest; text between slashes is
  phonemes (`/h ii/`), `word=/…/` is an escape that keeps its word (`jingle=/j ii ng g uh l/`: the escape is sung, the word is
  its label, not a lexicon miss), and a `/` with a space after it is a phrase break (`"/h ii/ ! / /h ii/ !"`, `"hold me / close"`). Where the
  voice's own dictionary knows a word its count wins.
- **It is a control line** that makes two `vocal` calls (`vocal create ... notes=[...]`, then `vocal to_row
  play=true`) and loops on the loop row until stopped (the runtime keeps the clip id the second call answers and stops it when
  the program stops or ends, an edit takes the line away, or a looped song's lap turns); it fires where its time word says (no word: its section's first
  bar), and an edited melody under it is another control, so it sings again. `for N bars` is refused. A `when` guard
  after it is decided while the program plays.
- **The voice:** a formant preset (`robot`, `robot-choir`, `android`, `choir-aah`, `choir-ooh`, `soprano`,
  `bass-choir`, `talkbox`) or a neural name (`alto`, `tenor`, `baritone`, `bass`). **No neural engine is built yet**
  (`djhost::vocals::NEURAL_ENGINE_BUILT` is `false`): a neural voice is valid, `check` warns "the neural voice is not
  installed: falling back to the formant voice", the line plays on `robot`, and the canonical text keeps `voice=alto`.

- **A phrase by id:** `sing @hands_up on melody` sings a stock call, and `sing @tempest_such_stuff on melody` a line of verse,
  scripture, speech or work song first published before 1929 (CC0, each with its source and its meter as a step pattern):
  `docs/beatscript/phonemes.md`, "The public-domain phrasebook", and beatscript.app/phrasebook.

**Found phrases:** pick a line for its meter: https://beatscript.app/docs/found-phrases.md

The technique in full is `docs/beatscript/found-phrases.md` (borrow a public-domain line for its shape, keep the vocal low, cycle it buried and spotlit): trochees chant on the kick, iambs lilt on the offbeat, anapests and dactyls want a triplet feel; keep the vocal about A3 to F4; cross it onto the drums with `@ vox xfer drm_shk`. The MCP `list_examples` lists the lines (`phrasebook` true, or `meter`, `syllables`) and `genre_template` takes a `phrase`.

Canonical form: `sing "robots on the road" on melody bars=2 key=am voice=alto` (params sorted by name). Long note:
`docs/beatscript/ext-sing.md`.

### Assertions and `btw test`

A program can say what must be true of it.

```text
# the statics: proved by `check`
assert ends                              # a song ends after its last section
assert length == 1m4s ± 2s               # 32 bars at 120 BPM
assert length < 2m
assert key == am in intro                # a section with no `key` of its own has the program's
assert key == dm in drop                 # the `key dm` at the head of the drop
assert key != dm in outro
assert period == 1                       # nothing here comes round over more than a bar

# the measured ones: pending at `check`, judged by `btw test`
assert level A > 0.2 * B in drop         # the drums are not buried under the bass
assert level A < 0.01 in outro           # deck A stops with the drop (what is left is the tail of its last notes)
assert level master > 0.01

# the constraints: about the notes, decided by `check` (examples/beatscript/constraints.beatscript)
assert never A.snare beside A.snare      # no two snares on neighbouring steps
assert never B during sing in verse      # deck B rests while the vocal is sung
assert bass ends on 1                    # every section lands on its own tonic
```

- **Static assertions** are proved by `check`, and a false one is an error at the statement that says the actual value
  (so `eval` refuses the program): `assert ends`, `assert period == N` (the program's period in bars) or `assert
  period(drop) == N` (an evolve's), `assert length cmp clock [± clock]` (through the tempo map, so exact under a
  curve), `assert key == k [in SECTION]` (also `!=`). A clock is `3m30s`, `90s`, `4m`, `500ms`, unit glued; a
  tolerance goes only with `==` and `!=`.
- **Constraints** are about the notes, and are decided by `check` like the static ones: a false one is an error that names the
  bar and the step and what would fix it. `never X beside Y`: no step where X has a hit and Y has one on the next or the
  previous step, across bar lines, section seams and the turn of a `loop` (two hits on one step are together, not beside).
  `never X during Y`: no step where a note of X sounds while a note of Y does (a note sounds for its `len=` and is cut where
  its section ends); `sing` is the vocal, every note of every `sing` line from its onset until the voice lets go. `X ends on
  D`: in each place a section plays (or `in NAME`), the lowest note of the part's last onset is degree `D`, 1 to 7, of that
  section's own key. A subject is `A.snare` (one deck's part), `snare` (that part on both decks), `A` (a whole deck) or `sing`;
  `ends on` takes one pitched part (`bass melody harmony arp`). "Decided" means enumerated: `check` asks every step of the
  finite song, exactly and not by sampling, and it is not a pflow proof. A part with chance is judged on everything it may
  play (every `?` hit kept, every option of a `choose`), so the rule holds for every roll, and `ends on` refuses a part with
  chance. Notes: `docs/beatscript/ext-assert.md`.
- **Level assertions** (`assert level A < 2 * B in drop`) are measured. `check` notes them as pending; **`btw test`**
  renders the program offline once, with the engine's per-strip stems on, and judges each: deck A's level is the RMS
  of the `A` stem over the bars of the section (`master` the master mix). A section on the timeline twice is measured
  over both places. Exit code 1 from `djctl btw test` when one fails.
- **At the top of the program only** (never in a section, a `def` or a `repeat`); `in NAME` names a section, also one
  inside a stack. Assertions are part of the program (an edit is another CID) and are not imported.

```text
$ djctl btw test examples/beatscript/assert.beatscript
assert ends: the program ends at bar 32: pass
assert length == 1m4s ± 2s: the program is 1m4s: pass
assert level A > 0.2 * B in drop: A 0.110, B 0.129: 0.110 > 0.026: pass
10 assertions pass
```

Canonical form: statements print in order; a clock prints as minutes and seconds (`90s` is `1m30s`); a level
expression folds constants and prints with one space round a binary operator. Long note: `docs/beatscript/ext-assert.md`.

### Performed inputs

`knob(3)`, `pad(5)`, `fader(A)`, `held(key)` and `held(c4)` read the controllers **while the program plays**.

```text
#! beatscript 2
deck A: kick x ~ x ~ | snare ~ x ~ x | hihat x*8 vel=60..90
deck B: harmony chord(am7) | bass a1 ~ a1 ~

master: filter lp knob(1)                  # a knob opens the filter: 60 Hz at 0, 20 kHz at 1, and follows it from the first bar
every 1/16 bar: master: reverb knob(2) when held(key)    # read 16 times a bar, only while a key is down
every 1 bars: master: filter lp 200 + knob(3) * 7800

every 1 bars: deck B: level 1 - fader(A)
master: reverb 0.1 -> 0.2 + knob(2) * 0.6 over 4 bars

every 1 bars: master: delay held(key) * 0.5 when held(key)
every 2 bars: deck A: low 0 when pad(5)                  # pad 5 kills deck A's low end for the bar it is held on
at bar 8: master: reverb 0.9 when held(c4) and knob(1) >= 0.5     # middle C and the first knob past halfway

master: filter hp lfo sine 2 bars center=500 + knob(1) * 400 depth=100 + fader(B) * 100
master: resonance 0.1 -> knob(2) over 4 bars exp
every 8 bars: duck B under A.kick 6db when held(key)
```

- **An input is a number in 0..1**, quantised to 1/128 when read: `knob(N)` the N-th knob (the connected bank's: the
  MPK's eight, else the APC's nine faders), `pad(N)` a pad (0, or its velocity while held), `fader(A)` a deck's
  channel fader, `held(key)` the lowest key held over 127 (0 when none), `held(c4)` 1 or 0 for one key. It is a leaf
  of an expression: `200 + knob(3) * 7800`, `min(knob(1), 0.5)`, `when held(c4) and knob(1) >= 0.5`. `N` is 1 to 128 (a
  MIDI channel's controllers and notes) whatever is connected: a program checks, saves and shares the same on every
  machine. A knob nothing has moved, or nothing answers for (the page has none), rests at 1/2, as `check` reads it, so
  the filter above does not start shut; a pad, a key and a fader that is not there read 0. `btw get` says which.
- **Where an input may stand: control values and control guards, and nowhere else.** A net is compiled once per
  section, so an input in a pattern, a count, a time, a tempo, a `let`, a def call's guard, a mix item, an assertion
  or an `evolve` rule is an error at the call that says what to write instead.
- **An input written alone as a cutoff covers the filter's range** on its own log curve, up a higher cutoff:
  `filter lp knob(1)` is 60 Hz to 20 kHz, `filter hp knob(1)` 20 Hz to 8 kHz. Arithmetic puts the value in hertz:
  `200 + knob(3) * 7800`.
- **A value follows its input.** A control line with a plain value that names an input and no time word fires on every
  bar from its first, not once, and sends a call only when the value has moved a step. In a `section` it follows each
  bar of it and stops with it. A line with a time word (`every`, `at`, `on phrase` ...), a `for N bars` line, a line in a `repeat`, a ramp, an
  lfo, and a line whose only input is a `when` keep the meaning they had.
- **Read when due.** An input is read each time a line that names it is due, so a line due every 1/16 bar (the time
  word that exists: 121 ms at 124 BPM, `every 1 beats:` 0.48 s) hears a knob turned inside the bar. Lines due
  together read it once and agree. A ramp that names an input reads it again at each bar line; an lfo's `center=` and
  `depth=` follow it. A `when` that is false when the line is due means it does not fire this time (a line that
  follows says so once, and goes on when it holds: `every 1 bars: ... when held(key)` is for lines that repeat on
  purpose).
- **Every value read is logged** (program bar, place in the bar, input, value) into a **performance**, an object with its own CID
  (`{type: djrust.performance, program: <CID>, inputs: [...]}`). `btw replay <CID>` evaluates the performance's
  program afresh and feeds the log to it in place of the controllers: the same calls with the same numbers, call for
  call. `perform {do: start | stop | save | list}` takes and keeps takes.
- **`btw test` and the offline render read every input as 1/2** and say so: a duck or mix with a performed `when` is
  in the render when the guard holds at 1/2.

Canonical form: one spelling, `knob(3)`, `fader(A)`, `held(c#4)` (lower case, sharps); the performance is not in the
program's text or CID. Long note: `docs/beatscript/ext-perform.md`.

### Links to the ontology

Two more `#!` header lines say what a program is and link it to the BeatScript ontology (https://beatscript.app/ontology/ns#).

```text
#! beatscript 2
#! genre: https://beatscript.app/ontology/genre/techno
#! tags: acid, supersaw, four on floor, drop

tempo 128
key am

section intro 8 bars {
  deck A: kick e(4,16) | hihat e(5,8)
}

section drop 16 bars {
  deck A: kick e(4,16) | snare e(2,16,4) | hihat e(5,8)
  deck B: bass <a1 ~ c2 ~> | harmony chord(am)
}

arrange intro drop
```

- **`#! genre:`** names one genre and **`#! tags:`** terms of the vocabulary (instruments, grooves, scales, sections
  ...), by local name, label or IRI; every spelling is stored as the full IRI. They are comments to the language,
  **not in the program's CID**; a change of genre or tags is a new object of the same program, as a new licence is.
- **Checked against a cached snapshot** of the vocabulary (`crates/djhost/data/beatscript-ontology.json`, 265 terms
  of the BeatScript ontology, compiled in, refreshed offline from the in-repo release by `tools/ontology/refresh.py`;
  nothing fetches anything). An unknown or
  wrong-kind term is a **warning** in `check` and `eval` (a comment must not stop a live program) and an **error** in
  `save` and `import` (a typo kept in a saved object would be a lie that persists), with line, column and a
  did-you-mean. **At most 64 tags**: each is looked up (about a millisecond), so the 65th is an error and the rest are not looked up.
- **`btw jsonld`** (and `djctl btw jsonld FILE`) exports the program as JSON-LD: `@id` the share form of the program
  CID, `@type djrust:Program`, `genre`, `tag`, `key`, `tempo`, `sections` (as the arrangement places them, stacks
  included), the rights, `forkedFrom`, `credit`, and one `euclid` node for each distinct `e(k,n,rot)` of the expanded
  song (a song's part that every section shadows plays nowhere and is not counted), linked to the ontology's Euclidean rhythm class. Keys are sorted and nothing volatile is in it. A saved
  program keeps `<object cid>.jsonld` beside its `.beatscript`.
- **`btw terms [kind]`** lists the vocabulary a header line is checked against.

Canonical form: nothing in the program's canonical text changes; the stored form of the links is full IRIs, tags
sorted and unique, genre first. Long note: `docs/beatscript/ext-linked.md`.

### Pattern sugar

Short ways to write patterns the language already has. Each one is **written out to its longhand before the pattern is
played**, so it compiles to exactly the nets of that longhand, and `btw same-as` says so (`crates/beatscript-api/tests/sugar.rs`
proves every row against the longhand given here). The ideas come from TidalCycles and Strudel; the names, the
semantics and the code are BeatScript's own.

```text
#! beatscript 2
tempo 124
key am

deck A: kick x~3 x | hihat ply([x ~ x x], 2) | clap poly(8, [~ ~ x])
deck B: bass c2~3 e2 | melody iter(off([a4 ~ c5 ~ e5 ~ ~ ~], 1, 12), 4)
```

| sugar | longhand | idea from |
|---|---|---|
| `c2~3` | `c2 ~ ~`: the step takes three slots, rests in the two after it | mini-notation `@` (elongate), spelt `~` here |
| `x?` | `x?0.5` (`x??` is `x?0.5?0.5`) | mini-notation `?` with no number |
| `poly([a b], [c d e])`, `poly(4, [a b c])` | each pattern walked `n` steps a bar as an alternation of the bars it takes to come round, stacked: `[[a b], <[c d] [e c] [d e]>]` | polymeter `{a b, c d e}` and `{..}%n` |
| `ply(p, n)` | `*n` on every step of `p`, groups and alternations opened: `ply([c2 <e2 g2>], 3)` is `[c2*3 <e2*3 g2*3>]` | `ply` |
| `palindrome(p)` | `<[p] reverse(p)>` | `palindrome` |
| `iter(p, n)` | `<[p] rotate(p, -s) ... rotate(p, -(n-1)s)>`, `s` the steps of `p` over `n` | `iter` |
| `off(p, n)`, `off(p, n, k)` | `[p, rotate(p, n)]`, `[p, transpose(rotate(p, n), k)]` | `off` / `superimpose` |

- **`~n` is glued to the step** and comes last, after its marks, repeat and chance: `x*2?0.6~4`, `c2!:2~3`. A step takes
  one (`c2~5`, not `c2~2~3`); `n` is 1 to 64.
- **There is no replicate mark.** `x:3` is the ratchet ("Expression marks"): three strikes inside one sixteenth step, not
  three steps. Three slots of `x` are written out, `x x x`.
- **Inside `< >` a term is a bar**, so `~n` there is an error that says so (three bars of `c2` are `<c2 c2 c2 g1>`). In
  `choose [..]` an option is one pick: give it weight, not `~n`.
- **`~n` is time, not sound**: the note is as long as the part's `len=`, like every step; a note that sounds for its whole
  slot is not sugar (see the notes below).
- **`poly`** with `n` left out walks the first pattern's step count; each pattern of `m` steps comes round in
  `m / gcd(m, n)` bars (at most 64). An alternation inside a `poly` pattern turns once per round of that pattern.
- **`ply` plays written steps**: a generator or transform inside it is an error with the way out (`transpose(ply(riff, 2), 5)`).
  A step that would go past `*64` is an error.
- **`iter(p, n)`**: `n` divides the steps of `p` (`iter([x ~ x], 2)` says which counts do), at most 64 steps.
- **`off`** moves the copy with `rotate`, so a copy pushed past the bar line comes back at its start, in the same bar.
- **Seeds are the longhand's**: a part's implicit seed (of chance and `markov`) is taken from the part written out, so
  `ply([~ x?0.5 ~ x], 2)` drops the same hits as `[~*2 x*2?0.5 ~*2 x*2]`. A program with no sugar keeps every CID it had.
- **An arrange mutation turns the longhand's steps**: `~ acid rot 1` on `let acid = c2~3 e2` rotates `c2 ~ ~ e2`, as it
  would the pattern written out.
- The longhand of one part is at most 16,384 steps, each copy counted at what it holds (`poly(64, [poly(64, [x x x x x x x])])`
  is an error, not a wait).

Canonical form: the sugar as typed (`c2~3`, `ply([x x], 2)`): formatting never writes it out, so `c2~3` and `c2 ~ ~` are
two programs with one song. The bare `?` is the exception: it prints `?0.5`.

#### Not sugar yet (design notes)

These need something the nets or the language do not have today, so they are notes, not syntax:

- **`sometimes` / `often` / `rarely` (apply a transform to some events).** The longhand would pick, event by event, between
  `p` and `f(p)`, and BeatScript has no function arguments and no way to pair an event with its transformed self (`rotate` moves it).
  A seeded `maybe(p, 0.5)` covers "drop some"; "change some" needs a per-event choice term: `either(p, q, 0.25, seed)` keeping
  each event of `q` where the hash keeps it and of `p` where it does not, both read on the same grid.
- **`struct` / `mask` (one pattern's rhythm on another's notes, or gating).** Needs the *extent* of each step (where its slot
  ends), which the evaluator does not carry: events are onsets. Adding a slot width to each event is an evaluator change,
  not a new net kind, and makes both expressible (`mask(riff, [x ~ x x])`, `struct(e(3,8), <c2 eb2>)`, `c2(3,8)` as Tidal
  writes Euclid on a note).
- **`chunk(n, f, p)`.** A transform applied to a different quarter each bar: needs a time-window term (`slice(p, i, n)`) and
  function arguments; with the window it is `<[f(slice 0) slice 1..] ...>`.
- **A `~n` that sustains** (a note as long as its slot). Each hit has a length in the net already (`len=`); a per-step
  length would be an event field, not a new net kind, but it changes which longhand exists, so it is kept apart from `~n`.
- **`jux` (a transform played in the other ear).** A pan per event is a new net field: the parts of a deck share one strip,
  so today it is two parts and a `pan` on the mix.
- **`range` / `segment` (continuous signals sampled into steps).** BeatScript's `lfo` and shapes already move controls over time
  ("Shapes and lfo"); sampling a signal into note velocities or pitches would be a new kind of term.
- **`swing` / `nudge` per pattern.** `swing` is a song setting and the grid is sixteenths ("Time finer than a bar"); a
  per-part shuffle needs the net's timing to vary within a part.
- **Top-level `,` stacking** (`x ~, ~ x` without brackets): reads ambiguously next to `|` and call commas; `[x ~, ~ x]` says it.

### Call and response

One deck plays a call, another plays what the call leaves. `rests(p)` is a hit on every step where `p` rests, and
`rests(p, q)` is `q`, kept only on those steps. Write the call once, as a `let`, and give it to both decks: a `let` is text,
so editing it moves both decks together, and so does an arrange mutation of it.

```text
#! beatscript 2
tempo 122
key am

let call = x ~ ~ x ~ ~ x ~
deck A: kick call
deck B: clap rests(call, [~ x ~ x ~ x ~ x]) | hihat rests(call) vel=50..75
```

The kick is a tresillo (steps 1, 4 and 7 of eight). The hat plays every step the kick leaves (2 3 5 6 8). The clap plays 2, 6
and 8 of its own pattern: its fourth step falls on the kick, so it is lost. Change `call` and both answers move.

- **It is a pattern call like `reverse`**, read when the pattern is: no statement, no change to the grammar, and nothing
  passes between the decks while they play. A deck's net stays one ring with its own token, because a net is compiled once
  and a pattern cannot read a token that arrives while it plays (see "Performed inputs"). The answer is exactly the hits a
  person would write for the complement, so its nets and CIDs are the longhand's (`crates/beatscript-api/tests/answer.rs` proves it
  with `same-as`). The canonical text keeps `rests(...)` as typed, as it keeps the sugar.
- **The steps are the call's own**, not the sixteenths. A pattern of several terms has that many steps: `x ~ x ~` is four, and
  its answer is two hits, where an answer on the sixteenths would be fourteen. A pattern that is one term is looked through:
  `[..]` is what is inside it, `<a b>` the term this bar plays, `*k` is `k` times the steps it repeats, `e(k, n)` is `n` steps,
  `arp(chord, mode, n)` is `n` (8 when left out), and a transform has the steps of its pattern (`slow(p, n)` the steps in one
  bar of it, which must be whole). Anything else, a note, a rest or a chord, is one step.
- **A step sounds when something starts in it.** A group, a repeat, a ghost, a flam, a ratchet and a chord are each one onset
  in their step. A note held by `len=` or slid into does not hold the steps after it.
- **A call that is silent in a bar is one rest**, so the bar is answered whole: `rests([<call ~>], reply)` plays all of `reply` in
  the bar the call is silent, and the second form is the way to answer a whole bar. (A pattern given as an alternation is
  written in brackets, as everywhere a call takes one.)
- **The call is read as this part reads it**: its key, octave and bar, for onsets only. A numeral or a chord in the call of a
  drum part is an onset, not an error, and does not move the part's voice leading. Give both decks the same cycle: a part with
  `/N` reads the call in its own cycle. The answer comes round when the call and the reply have.
- **The call has no chance in it.** Each part rolls its own dice (its seed is of its deck, its role and its text), so a `?`,
  `choose`, `maybe`, `degrade` or `markov` in `p` would be a different call on each deck. It is an error that says so and shows
  the way out: write the call without chance and put the chance on the part that plays it (`snare maybe(call, 0.8)`). The reply
  `q` may have chance, and an arrange mutation of the call (`thin`) rewrites its text before it is read, so both are fine.
- **A call that sounds on every step answers nothing**, which is a warning: leave a rest in it, or answer another pattern.
  `rests` with no arguments, three, or a `name=value` is an error that gives the form.
- **An answer is a function of the call's text, not of what is playing.** Mute the call's deck and the answer is still in its
  gaps. A sung line holds a note to the next onset (at most half a bar), so an answer in the gaps of a sung part sounds under
  the held vowel; a bar of silence in the call gives an answer that stands alone.
- **`rests` is not a reserved word**: a `let` may be named `rests`, and the call is the word before a `(`. As a typed
  port, `rests` keeps a pattern a rhythm.

Canonical form: `rests(call)`, `rests(call, [~ x ~ x])`, as typed. Long note: `docs/beatscript/ext-answer.md`.

## Categorical notes

Three pieces of the language have an algebra, and the language keeps to it where it can be kept: the transforms of an
`evolve`, the composition of sections with `stack` and `then`, and the typed ports of a `def`. What each promises is
written here, with what it does not.

**Evolve's transforms are a monoid.** A transform is an endomorphism of the realised hits of a part (a part to a part
of the same shape). Writing them in a rule, `rotate 1 | transpose 5`, is composition in order, which is associative
and has an identity (`transpose 0`, `rotate 0`, `octave 0`): the transforms of one part form a monoid, and a rule
whose transforms do not mention `gen` makes generation n the n-fold composite `f^n`, iteration in that monoid. It is not
commutative (`rotate 1 | reverse` is not `reverse | rotate 1`). A rule that does mention `gen`, or `thin`, which hashes
the generation, is a *sequence* of endomorphisms (`f_1, f_2, ...`) rather than one repeated, and generation n is
`f_n(...f_1(gen 0))`: still a composite, no longer a power. Nothing about evolve changed with `stack` and `then`.

### Composition: stack and then, as a structure

A song is a composition of sections, and the two ways of composing them are in the language (above): `arrange intro
stack(then(riff, riff), verse) coda`.

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

**What `then` of parts means.** Over parts the associativity law is on the text, and the music follows the canonical text:
`then` is a *sequence*, so a nested `then` flattens into it (`then(then(a, b), c)` is `then(a, b, c)` is `kick <a b c>`: bar 0
plays `a`, bar 1 `b`, bar 2 `c`, bar 3 `a` at its own second cycle when `a` alternates). It is never the alternation of an
alternation the nesting suggests (`<<a b> c>`, whose inner part advances once per visit): that is a different program, written as
the alternation it is, with its own CID.

**What they do not.** Equal *music* is not equal *text*. The interchange law, `stack(then(a, b), then(c, d))` equal to
`then(stack(a, c), stack(b, d))` when `|a| = |c|`, holds for what is played (the same notes in the same bars, so the
same nets of the song written out longhand) but it is not normalised: the two are different programs with different
CIDs. A section cut in two halves is not the section, because every section has its own clock (`at bar N`, `every`
and its alternations count from its first bar). A `let` names a composition and a name is opaque to the laws:
`let both = stack(a, b)` and `stack(a, b)` are not the same text. And the laws are about the shape of the
composition, not about the sounds: they say nothing about whether two sections sound alike.

### Ports and cospans

A `def` with ports is an **open system with an interface**, and `wire` **composes** two along the interface they
share. The nine port types form a preorder (a rhythm fits a bass, a melody or drums; each of those fits a pattern),
and an interface is a finite set of typed ports, each named by its type. A def with `in: X out: Y` is a *cospan* `X →
N ← Y` (its apex has one cell for each port), decorated with what its body says: its lines, with a hole for each input
and for each parameter, and its **emit function** from the values of the inputs to the values of the outputs ("put the
inputs into the `emit` statements"). A wiring from one def to another is a set of pairs (output, input) with the types
fitting, a function on the inputs (each fed once) and arbitrary on the outputs (an output may feed many inputs, or
none: copying and discarding, as in a cartesian structure, and never a merge). The composite is the cospan whose apex
is the **pushout** of the two apexes (the two cells of a wired pair become one), with the lines of both and the
composed emit function. Imports are pinned by CID, so what is composed is exact modules.

**What the language adds to cospans: no feedback.** In the full cospan category a wire from a def's output to its own
input is a trace, and fine. Here it is an error (`the wires make a loop`), because a value that feeds itself has no
meaning for a macro that expands in finitely many steps: the wirings are the *acyclic* ones, a (cartesian) symmetric
monoidal category of modules, not the compact closed one. That is the totality rule of a `def` that calls itself,
again. The laws the tests hold (in `crates/djlive/tests/ext_ports.rs`): wiring is associative (any bracketing of a
chain, any cut of it, a saved snippet being one bracket, expands to the same lines); the order of the `wire`
statements does not matter; the module `in: t out: t { emit $in }` is the identity; independent modules compose in
parallel and the order of their calls is the braiding (it changes the order of lines, not the lines or the nets); the
value at an out port of a composite is the composite of the emit functions; a loop is reported exactly when the wiring
has one; the canonical form is a fixed point; and a def with no ports is the macro it was, with the same text, CID and
expansion.

## How long a program can be

A **net** is at most **64 bars**: the piano roll that makes a deck's net (`djrust::roll`, from the Swift app) rings one place
per sixteenth step per part and refuses more than `MAX_STEPS = 1024` (64 bars). It is a size bound, not an index width
(a step is an `i64` there, a hit's step in its bar a `u8`): a net is a place, a transition and two arcs per step per part,
and the raw-nets envelope that carries it is JSON. A program is not a net. A deck whose parts take longer than 64 bars to
come round (an 80-bar alternation written longhand) is **chained**: cut into stretches of 64 bars, each its own net, loaded a bar
ahead and landing on the bar line the one before ends on. A section longer than its net is not chained (a 4-bar pattern
loops for 200 bars as one net); a section whose alternation is longer than 64 bars is; a program with no sections is
given a timeline of the bars its decks take to come round together, and **loops**, its `at bar` and `every` lines counting
program bars as they always did. The most one alternation may take to come round, and the most a timeline may add up to,
is **16,384 bars** (9 hours at 120 BPM). What a stretch costs is the same whatever the program's length; the cap is not
raised because a longer *net* costs more than linearly in what matters live:

| one net of | JSON (8 parts, 2 decks) | `check` | envelope | CID | deck load | peak RSS | render speed |
|---|---|---|---|---|---|---|---|
| 64 bars | 1.1 MB | 2.9 ms | 8.9 ms | 16 ms | 0.60 s | 223 MB | x34 real time |
| 128 | 2.2 MB | 4.4 ms | 16 ms | 34 ms | 1.03 s | 426 MB | x32 |
| 256 | 4.6 MB | 8.4 ms | 34 ms | 70 ms | 1.89 s | 784 MB | x28 |
| 512 | 9.2 MB | 16.5 ms | 69 ms | 140 ms | 3.57 s | 1.58 GB | x22 |
| 1024 | 18.8 MB | 31.7 ms | 137 ms | 285 ms | (not loaded) | | |

(Measured with `cargo run --release -p djlive --example net_cost`, and `djctl btw render` on a build of the roll with the
cap lifted, on an M4 Pro at 124 BPM; the deck load is the wall time of rendering the first four bars.) Compiling is
linear and cheap; the engine's load is about 7 ms and 3 MB a bar, so a 256-bar net takes about a bar of real time to
load at 124 BPM and a 512-bar one two, on the window's thread, and the audio loop gets slower per second of sound as the
net grows. **Chained**, the same 512 bars load in 0.61 s (the first stretch) and render at x33.5, so a long track costs what a
64-bar one does at any moment. (The renders hold every stretch at once, so their peak memory is higher than a live
run's, which holds one stretch a deck.) Raising `MAX_STEPS` itself would be a change in the public engine and in the Swift
parity it keeps, for no gain a chain does not give.

Two extensions lengthen a net without an alternation, and both stay under the same 64 bars. A part with a **cycle of its own**
(`hihat/12`) comes round after `lcm(N, 16) / 16` bars (`/12` three, `/10` five, `/63` sixty-three), and a deck whose parts with
cycles take more than 64 bars to come round together is an error that names the parts and the way out (a long alternation
with no cycle of its own is still chained, as above). A part with **chance** is unrolled over the bars it needs to differ: 16
bars in a program with no sections, the section's length (up to 64) in a section, rounded up to whole copies of its
alternations and cycles. `check` reports the period of the program (`period: 30 bars = 480 steps (...)`) and the bars each
chance is unrolled over.

### Bounds on the text

The language is total, and so is the reading of it: every recursion in the reader and every multiplication in the
expander has a bound, and a program that crosses one is an error with a span, never a stack overflow, a hang or a
gigabyte. (The text of a share link is read by the editor thread, the control socket and `btw open`.)

| what | bound | where it is said |
|---|---|---|
| nesting of groups `[ ]`, alternations `< >`, calls and their arguments, `deg( )`, sections | 64 deep, counted together | an error at the opener that is too deep (a section's is the last thing read) |
| operators in one expression (`+ - * / %`, comparisons, `and`, `or`, `not`) | 128 | at the operator past it |
| a pattern's terms, with the `let`s in it put in place | 16,384 terms and 128 levels deep, worked out from the text before anything is built | at the `let` (or the part) that crossed it |
| events one part plays in one bar | 4,096 | at the term that crossed it; the work stops there |
| terms one bar of a part reads (nested `*N` of rests make no events) | 8,192 | the same |
| repeats and calls: the text they write out, counting each copy and each call a little even when it plays nothing | about 2 million tokens | at the repeat or call; the first `nest more than 8 deep` stops the expansion |
| lines | 4,096 | at the line (at the `arrange` when a section placed again and again is the cause) |
| a chain of `let`s of compositions | 128 | at the name past it |

`[[[x*64]*64]*64]*64` is 16.7 million hits and `let b = a a`, `let c = b b`, ... is 2^n terms in n lines: both were
built before anything counted them, and one more level never ended. A bar holds 16 steps, so 4,096 events is far
more than a part can use.

## Sets: programs of programs (design note)

A **set** is a program whose body is other programs, played one after another with the mixes between them. It is sequential
composition (∘) at program level: a set is a program, so it has a CID, a canonical form, a share link, a timeline and an end
like any other, and a set can `play` a set.

```text
set {
  play hand-dubstep-140-v4
  mix 8 bars with filter_exit
  play matrix-130
  mix 16 bars with crossfade, tempo 140 -> 130
  play deep-house-60 until 20m
  play gen@2 techno seed=42
  mix 16 bars with crossfade
  next seed=+1
  mix 16 bars with echo_out
  next seed=~
  play yellow-rose-174
}
end
```

- **`play NAME|CID [until WHEN]`** names a saved program (a name, in quotes if it has spaces; a CID; or the end of one, in quotes when it starts with a digit). The
  canonical text pins it by CID, as `import` does (`play zQm...`), so the set's CID covers its tracks: change a track and the
  set is a different set; a rename changes nothing. A name that matches no saved program, or two, is an error.
- **`play gen@V GENRE seed=N ...`** takes a generator call as a track, besides a saved program's name or CID: the program the
  call prints ("Call the generator"), pinned by its version.
- **`next seed=+N [until WHEN]`** (N >= 1) or **`next seed=~`** is a track: the track before it (a `play` of a generator call, or
  another `next`) with its seed stepped by the station rule ("Radio: stations"): `+N` counts on, `~` draws the seed from a hash
  of the CID of the track before. A `mix` may sit between. `next` after a saved program (a name or CID) is an error: there is
  no seed to step. The canonical text keeps `next seed=~` as written (it is structural: in the text, so in the CID); `check`
  reports the call each `next` resolved to and its CID (`set.tracks[i].call`, `.cid`), in a note at it: `next: gen@2 techno
  seed=43 -> z4EB... (seeded@2)`.
- **`mix N bars with TRANSITION [, tempo A -> B]`** sits between two tracks (a `play` or a `next`) and overlaps the last N
  bars of the outgoing track with the first N of the incoming one. Without a `mix` the next track starts where the last ends (a
  cut). `cut` is a mix of no bars (it takes no overlap: N is ignored and `check` says so).
- **`until WHEN`** cuts a track: a time (`20m`, `90s`, `1h`, `3m30s`, `1h30m`, read at the track's own tempo, through its tempo curve when it has one), `bar N` (of the track's own count),
  a section name (up to where that section **starts**) or `after SECTION` (after it ends). A track that is shorter than its
  `until` just ends. A track with an `end` (or an `arrange` that is not `loop`ed) plays to it; a flat or looping track with
  neither needs an `until`, and `check` says which one.
- **`end`** after the set ends it after the last track (the default); `end at bar N` cuts the whole set; `loop` starts it over.
  `use restricted` is accepted, and required when a track uses restricted assets (a set is as restricted as its tracks).
- A set holds the `set` block (its `play`, `mix` and `next` lines), `end`/`loop`, `use restricted` and comments, nothing else: a
  deck line, a `section` or a `station` block next to it is an error that says so.

**What a mix does on two decks.** The engine has two decks, and a track plays its parts on both, so two tracks cannot each have
a deck to themselves for the overlap. The mix therefore puts both tracks' parts on the same two decks, one bar at a time (N
one-bar stretches of the timeline), with each deck's net rebuilt every bar:

- a part the other track has no counterpart of (same deck, same role) is **faded by velocity** along the transition's curve:
  `crossfade` out 1 - t, in t; `filter_exit` the outgoing falls away fast, (1 - t)², the incoming rises at 1.5 t; `echo_out` the
  outgoing decays as (1 - t)³ while the incoming rises linearly (t is the bar's middle, from 0 to 1 across the mix);
- a role both tracks have on a deck (both have a `kick` on A) cannot be two parts of one net, so the deck **swaps** it, bass-swap
  style, from the outgoing track's to the incoming track's at the middle of the mix, at full level;
- the **tempo** ramps bar by bar from the outgoing track's to the incoming track's (or from A to B when the `mix` says
  `tempo A -> B`), so both play at one tempo while they overlap;
- tracks that each use one deck (A and B) overlap on their own decks, and the outgoing deck stops when the mix ends: the deck
  alternation of a classic mix. Two-deck tracks overlap as above, and a deck stops when no later stretch has a net for it.

These are approximations by velocity and swap: the engine's real filter and echo moves act on a whole deck, and during a mix
both tracks are on it. They are what two tracks on two decks can honestly do, and what the compiler can prove (`check` knows
every bar's levels), not a recording of a hand mix.

**What `check` proves or plans** (per mix, in the answer and as warnings):

- **tempo**: `mix 3→4: 130→122 over 16 bars` is the plan; a ramp that moves more than 3 BPM a bar, or a mix whose tempos
  differ by more than 8%, is a warning that names the tracks and a gentler length;
- **key**: each track's key (its `key`, else unknown) as a Camelot code (A minor is 8A, C major 8B). The same code, one step up
  or down the wheel, or the relative major or minor are compatible; anything else is a warning ("mix 2→3: 8A → 3B: keys clash");
  an unknown key is said, not guessed;
- **length**: each mix must fit (N <= each track's length, and the two mixes at either end of a track must not overlap each
  other), and the total length, in bars and in minutes at the tempos in play, is reported with a per-track timeline
  (`set.tracks`: where each starts and how long it plays, its tempo, key, whether it is cut) and the mixes (`set.mixes`).

**Seen and heard as one timeline.** A set compiles to an ordinary arranged program: one segment list and one control clock, so
the runtime, `end`, `loop`, the ended state, hot reload and anything that seeks or reads an arranged program treat it as one
timeline. A track's control lines (filters, `do` calls, ramps) move to the track's place in the set and run only while the
track does (`every N bars` stops at the track's end). The playhead sits on the `play` line of the current track and on the
`mix` line during a mix, so the follow-scroll goes track to track and mix to mix. What is **not** lit are tokens inside a
track: their text is the track's, not the set's. (The window shows the set's text; the track opens with `btw load`.)

**A recorded set as a set.** `beatscript set-program <recording>` (and the booth's `record program`) writes a recorded set back
out as a set, in the syntax above. Each tracklist line that is a saved program becomes a `play`, cut with `until bar N` where
the next track came in. The moves into it become one `mix`: its bars run from the incoming track's mark to the last move, a
move with no set spelling (a bass swap, a loop roll, a phrase sync, a vocoder handoff) is written as a `crossfade` under a
`# performed:` line, and `tempo A -> B` is added when the master tempo moved. A generator track (`techno:7` with nothing
changed) is played as `arrange from seed(techno, 7)`, which plays the generator's notes. Anything else (a mashup, a loop
set, a sample) is a `# not a program:` line, and its bars are said: the track before plays on through them, or, before the
first program, they are a `# gap:`. A sample makes the set `use restricted` and private. Beside the program it writes the
**take**, `<cid>.take.json`: the tracklist, the replay and the controls it was played with, naming the program by CID as a
performance does. A program is in a tracklist only when the booth marks it (`record mark`, kind `track`, its program CID).

**Share links.** The canonical text pins saved tracks by CID, and a generator call or a `next` is its own text, so a link of a
set is the set. A link carries its saved tracks' canonical texts (`*` section, each one checked against its CID on opening),
however long the link is, so it opens where no track is saved ("Share links").

**Sets and the extensions (what the two bodies of work decided together).** A set is checked by the whole compiler: each track
is compiled as any program is, with its arithmetic, tempo curve, chance, `stack` and `then`, ports, `assert`s and `#! genre:`
lines, so a set of such tracks checks, has a CID of its own and pins each track by the CID of its canonical text (a comment,
spacing or a header line of a track moves nothing; a number of its arithmetic moves the track and with it the set). What the
meeting settled:

- **One reading of a time.** A time is a number and the unit glued to it, `3m30s` as `3` `m` `30` `s`, in an `assert length` and in
  a `play ... until`: `20m`, `90s`, `1h`, `3m30s`, `1h30m`, `0.5h` (`h` is a unit the lexer reads everywhere, and only an
  `until` takes it). The canonical text prints the shortest exact spelling (`1200s` is `20m`, `3600s` is `1h`, `5400s` is
  `90m`, `210s` stays). The upstream branch read `20m` as one token; the extensions' units were already tokens of their own.
- **The words.** `set` starts a statement only before `{`, `until` follows a `play` and `mix N bars with` is a set's: none is
  reserved (a `let`, a def or a section may be called `set`, `until` or `mix`, as `mix` already could, where no `let` of that name
  stands in its place), the highlight shows them as keywords where the set reads them, and a typo of `set` at the start of a
  line is hinted. A program with `set` holds nothing else (`let`s, decks and sections belong to the tracks).
- **The tempo.** A set has a tempo map, as every program does (docs/beatscript/ext-tempo.md): each track's tempo, its curve too, where it
  plays alone, and each mix's ramp (`tempo 140 -> 130`, or from the outgoing track's tempo at the bar the mix starts to the
  incoming one's where it ends) over the mix's bars. The runtime follows it like any program's curve (the deck's tempo moves
  smoothly through the mix, not a step a bar; the nets still carry each bar's tempo), `length` reads it
  (`length: 4m10s with the tempo curve, 64 bars`), the plan says when only a ramp changed, and `until 20m` counts the track's seconds
  through its curve when it has one. A set whose tracks all play at one tempo has a constant map and is not driven, as before.
- **Lines a track brings.** A track's control lines move to its place and run only while it plays: bar-line timings by the
  `window` of upstream, the sub-bar ones (`every 2 beats`, `at bar 1 beat 3`) by the same window, an `lfo` stops where its
  section ends on the set's clock (or where the track ends, for a line outside a section), and a `duck` with no `for` is put back
  when its track ends.

## Radio: stations

A **station** is an endless, deterministic program of programs: position N of a station is always the same track. A station is
itself a BeatScript program (CC0, with a CID) that says where the walk starts, how it steps from track to track, and the forms
the tracks take in turn. Listeners do not steer a station; they tune it and lock its knobs.

```text
#! name: techno radio
#! license: CC0-1.0
tempo 128
key am
station {
  from gen@2 techno seed=42
  next seed=~
  forms seeded passacaglia rondo fugue
}
```

- **`from gen@V GENRE seed=N [length=L]`** is the first track's generator call, and it is required. `section=`, `part=`,
  `bpm=` and `key=` are refused: a station's tempo and key are its `tempo` and `key` lines, and where it has none, the ones the
  call prints.
- **`next seed=+N`** (N >= 1) or **`next seed=~`** is the step from one track's seed to the next: `+N` counts on, `~` hashes the
  CID of the track before's generator call (`arrange from gen@V GENRE seed=s`), so tempo, key and form do not move the walk.
  Default `next seed=+1`.
- **`forms NAME ...`** is the form rotation: track N plays `forms[N mod len]`, of `seeded` (the generator's own arrangement),
  `passacaglia`, `canon`, `rondo`, `fugue`, `theme-and-variations` and `cross`; a name may come twice. Default `forms seeded`.
  Each pass of the rotation is a chapter, and its first track's seed is the chapter's theme, bound to each form's theme role (a
  ground, a refrain, a subject), so it recurs through the chapter while the form's other role takes the track's own seed.
- **A track** is `tempo`, `key` and one `arrange from` line, under a `#! station:` header line (and from track 1 a `#! from:`
  line); between two tracks a **segue** is the `cross` form of their two seeds, 192 bars. Each has a CID of its own.
- **Knobs**: `genre`, `seed`, `step`, `forms`, `tempo`, `key`, `length`. A retune sets one and keeps the position N; one notch is
  a neighbour (seed +-1, tempo +-2, key one step round the circle of fifths, `+1` and `~`, the forms rotated by one, length one
  step). A lock holds a knob: a locked knob has no neighbours and is not retuned.
- A station holds the `station` block, `tempo`, `key` and comments, nothing else, as a set does. `#! stations: v1` is the version
  (the default; any other value is an error). The station's CID is of its canonical text; comments and `#!` lines are not in it.

**Radio:** a station is an endless program, track N always the same: `station { }` holds `from gen@2 techno seed=42` (the
first call), `next seed=~` (the step) and `forms seeded rondo fugue` (track N's form, in turn); the MCP `station` tool and
`POST /api/station` give track `n`, its segue and its neighbours: https://beatscript.app/docs/radio.md

## Restricted sounds: local samples (design note)

A private set may play sounds it has no right to give away (your own Minecraft note-block samples, a sample pack
you bought) without the program, a link, the repo or any public store ever holding the bytes. The rules, before the
code:

```text
use restricted                                       # the program says it is private (or the file header does)
sample harp from "~/samples/harp.wav" license restricted-private else bright-pluck
sample kick2 from "sha256:9f2c..." license cc-by
deck A: bass riff inst=harp
```

- **A sample is a name for a file.** `sample NAME from "SRC"`: `SRC` is a local path or `sha256:<hex>`. The first time a path
  is read its bytes are hashed (SHA-256) and the path is remembered *on this machine only* (`<support>/assets/index.json`,
  never exported); the canonical text holds the hash, not the path (`from "sha256:..."`), so the program refers to the file by
  what it is. The bytes are never embedded in the program, a share link, the repo, a snippet or any store, and nothing here
  ever fetches one: you supply the file from your own copy. A hash with no file on this machine is an error that names the
  asset and says what to do (`sample harp from "/path/to/the/file"`), never silence and never a quiet fall back.
- **Each asset carries a license tag:** `cc0`, `cc-by`, `all-rights-reserved`, `restricted-private` (the default when none is
  said: the safe one). **A program's effective rights are the most restrictive of its own and its assets'** (cc0 < cc-by <
  all-rights-reserved < restricted-private). The tag is the program's own statement about the file; it is part of the text and so
  of the CID.
- **A restricted asset makes the program private.** Using a `restricted-private` sample (naming it in `inst=`) without `use
  restricted` in the program, or `#! visibility: private` in its `.beatscript` header, is an **error in `check`** at the `sample` line.
  Such a program cannot be shared (`btw share` refuses, naming the asset), cannot be made public (`btw visibility` and `save`
  with `visibility=public` refuse, naming it), and is not offered to a public store. **Forks inherit it** (a fork stays private
  and keeps the list), and so does a program that imports a snippet with restricted samples. A cc-by asset adds its credit like
  a CC BY import; an all-rights-reserved asset makes the program all rights reserved (a link may carry the hash; it has no bytes).
- **Reproducible where the files are:** the CID covers the asset hashes, so the same program on a machine that has the same
  files is the same program, and on one that does not it is a clear "missing asset" error.
- **A stand-in for the public:** `else VOICE` names an original synthesized voice (checked like `inst=`: `bright-pluck` today,
  the planned `blocky` palette's voices when they exist). `btw fork {substitute: true}` writes the fork with every restricted
  sample replaced by its stand-in (`inst=harp` becomes `inst=bright-pluck`, the `sample` line goes): a different program, a
  different CID, with nothing restricted in it, which can be made public and shared. A restricted sample with no `else` is
  refused, naming it.
- **Playing it:** the checker and the rights are done here. Playing a registered sample through the engine as an instrument is
  not wired (the engine has no sampler for a user file on a raw net): until it is, a part that names a sample plays its `else`
  voice (or the role's default) and the check says so in a warning, so it is never silent about it.

## Sounds

`btw instruments` (and `describe btw`) lists every **voice** the engine voices by itself (`wobble-lead`,
`hoover`, `detuned-saw`, `pwm-lead`, `acid-bass` and `acid-square` (a 303's; `acid` is `acid-bass`), the `voice-*` singers,
the stingers), the **family words** (`bass`, `reese`, `sub`, `pad` ...: a name that contains one plays that family's generic
voice; `acid-bass` and `acid` are a voice of their own, `acid-lead` is the generic saw bass), the drum **kits** and the sound
**palettes**. In a part:

| param | takes | does |
|---|---|---|
| `inst=wobble-lead` | a voice | the part's instrument. An unknown name is an **error with the nearest names**; a family word (`inst=reese`) is accepted with a **warning** that it plays the generic saw bass, and which voices have a sound of their own. `inst=acid-bass` (or `inst=acid`) is a 303: `!` accents, `^` slides, `cutoff=` `res=` `env=` `accent=` `drive=` turn its knobs |
| `kit=drums-808` | a kit | a drum part's kit (set once the net has landed) |
| `sound=synth:wobble-lead` | `own`, `synth`, `synth:NAME`, `inst:NAME` | the role's sound option on the deck, set once the net has landed (`DeckAction::SetRoleVoice`) |
| `route=td3` | `td3` | a tonal part also fills the TD-3 editor's sixteen steps (note, octave shift, accent, slide, rest) once its net lands; `do td3 write confirm=true` sends it (a dry run unless confirmed) |

A `sing` line (see "Sung lyrics") takes `voice=` from the same host: the formant presets (`robot`, `robot-choir`,
`android`, `choir-aah`, `choir-ooh`, `soprano`, `bass-choir`, `talkbox`) and the neural names (`alto`, `tenor`,
`baritone`, `bass`); the neural engine is not built, so a neural name plays on `robot` and `check` says so.

`master: resonance 0.7` / `deck A: resonance 0.2 -> 0.8 over 4 bars` set the filter insert's resonance
(`filter.res`); any other insert param is `do inserts set target=Master param=filter.mix value=0.5`.

## Example: a whole track

`docs/beatscript/hand-dubstep-140-v2.beatscript` (saved as `hand-dubstep-140-v2`; rendered offline to
`~/Workspace/llm-headline-loops/livecode/hand-dubstep-140-v2.wav`: 109.7 s, 64 bars at 140): an intro, a build whose
riser opens over eight bars and ends on the phrase line where the drop starts (with a `loop_roll` four bars
ahead of it by `before drop 4 bars`), a drop whose bass is the engine's `wobble-lead` voice (a checked
name) under a filter that opens and shuts once a bar, a breakdown, a second drop and an outro, from lets, one
def, sections and an `arrange`. The file is BeatScript 1 and stays so, since the tests pin its CID; `beatscript fmt` gives
the same track in BeatScript 2, with `#! beatscript 2` first:

```text
# hand-dubstep-140 v2: a whole track in one program.
# intro 8 · build 8 (the riser lands on the phrase line) · drop 16 · breakdown 8 · drop 2 16 · outro 8
tempo 140
key fm

# the parts, named once
let kicks = x ~ ~ ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~
let snares = ~ ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~
let hats = ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ x x ~
let wob_a = f1 ~ ~ f1 ~ ~ ab1 ~ f1 ~ ~ ~ eb1 ~ c1 ~
let wob_b = f1 ~ f1 ~ ~ ~ db1 ~ ~ ~ c1 ~ eb1 ~ eb1 ~
let lead_a = ~ ~ c4 ~ ab3 ~ ~ ~ f3 ~ ~ g3 ~ ab3 ~ ~
let lead_b = ~ ~ c4 ~ eb4 ~ ~ ~ db4 ~ c4 ~ ab3 ~ ~ ~
let changes = <chord(fm) chord(fm) chord(db) chord(db) chord(ab) chord(ab) chord(eb) chord(eb)>

# the drop's groove: the drums and the wobble bass on one deck, and a filter that opens and
# shuts once a bar (the wub), for as many bars as it is told
def groove(deck, bars) {
  deck $deck: kick kicks | snare snares | hihat hats | bass <wob_a wob_b> inst=wobble-lead
  repeat $bars every 1 bars {
    master: filter lp 300 -> 2800 over 1 bars
  }
}

section intro 8 bars {
  deck A: kick kicks | hihat hats vel=50..80
  deck B: harmony changes inst=warm-pad
  master: filter lp 400 -> 6000 over 8 bars
}

section build 8 bars {
  deck A: kick kicks | snare snares | hihat x*8 vel=50..100
  deck B: harmony changes inst=warm-pad | melody <lead_a lead_b> inst=pwm-lead
  # the riser: it opens over all eight bars and ends on the phrase line, where the drop starts
  master: filter hp 100 -> 5000 over 8 bars
  master: reverb 0.1 -> 0.5 over 8 bars
  # and the roll leads into it
  before drop 4 bars: move loop_roll on A over 4 bars
}

section drop 16 bars {
  master: filter off
  master: reverb 0.15
  groove(A, 16)
  deck B: melody <lead_a lead_b> inst=pwm-lead | harmony changes inst=warm-pad
}

section breakdown 8 bars {
  deck A: hihat x ~ x ~ vel=40..60
  deck B: harmony changes inst=warm-pad | melody <lead_a lead_b> inst=pwm-lead
  master: filter lp 6000 -> 1200 over 8 bars
  master: reverb 0.45
}

section drop2 16 bars {
  master: filter off
  master: reverb 0.15
  groove(A, 16)
  deck B: melody <lead_b lead_a> inst=pwm-lead | harmony changes inst=warm-pad
}

section outro 8 bars {
  deck A: kick kicks | hihat hats vel=40..70
  deck B: harmony changes inst=warm-pad
  master: filter lp 8000 -> 500 over 8 bars
}

arrange intro build drop breakdown drop2 outro
end after outro
```

## One song, two texts

`examples/trance-138.beatscript` is a 64-bar trance track (intro 16, breakdown 16, drop 24, outro 8) said with
lets, two defs, four sections and an `arrange`. `crates/djlive/tests/fixtures/trance-138.live` is the same song
written longhand: every part one 64-term `<...>` alternation with a rest bar wherever the part is out. They are
very different texts, and the song written out (`Compiled::song_nets`: one net per deck for the whole
timeline, a rest bar wherever a section does not have a part, velocity hashes taken at the song's bar) has
**the same net CIDs** as the longhand's own nets (`crates/djlive/tests/trance.rs`; `djctl btw nets FILE`):

```text
$ djctl btw nets examples/trance-138.beatscript
program z4EBG9j5MMozvDkT1gTck6ZKTsqXctuNA25bzD86dkhc6buYuuB
deck A  z4EBG9jG8nT2p449McscV7ZSCfmUJDiFxFU26JpZhJ9hfBFgeLD  (the song written out longhand)
deck B  z4EBG9j8hbXoGqrw8qMseuz7BhibdkMiee4USmUaSyPRgJjiiLJ  (the song written out longhand)

$ djctl btw nets trance-138.live          # the longhand, 17 577 bytes against the song's 3 184
program z4EBG9jGbdqJ73AvEDLrPE8g2LuigXEfsG4o9sAsXqgod7JX6CY
deck A  z4EBG9jG8nT2p449McscV7ZSCfmUJDiFxFU26JpZhJ9hfBFgeLD
deck B  z4EBG9j8hbXoGqrw8qMseuz7BhibdkMiee4USmUaSyPRgJjiiLJ
```

What plays live is the sections' own nets swapped on the bar lines (so a section edit reloads only that section),
which are four small nets a deck, not one 64-bar loop: same notes, different CIDs, because a net's CID is of the
whole loop. The longhand form is the normal form to compare in. Differences that would break the equality are
reported, not hidden: a part with another instrument in another section has no single longhand (`song_nets`
says which). The longhand as it was first written names `inst=supersaw`, which the engine has no voice for (it
played the default lead): the checker now refuses it, and the song uses `detuned-saw`, so the test compares
against the longhand with that one name changed (and asserts that name is the only difference).

## Seeing it play

The desktop app's `btw` window lights what is playing, in the style of Strudel and Sonic Pi, and fills the screen when asked.

- **Full screen:** the **Full** button, **Cmd+Shift+F** in the window, or `windows fullscreen window=live` (also
  `do windows fullscreen window=live`, `window=btw` works, `on=false` returns it). The editor gets the screen on a dark stage
  with large monospace text, and the header (section, bar, phrase, generation, the phrase's eight lights) stays on top.
  **Esc** returns. No other window goes full screen yet.
- **Token lamps:** each step token (`x`, `a2`, `e5`, `chord(am)`) lights up the moment its hit sounds and fades over a beat. A
  hit that came out of a `let`, a `def` or a transform lights every place it is in the text: the token in the `let` line,
  and the call site (`groove(A, 16)`, dimmer). Spans are kept through `let`, `def`, `repeat`, `section`, `evolve` and the
  transforms and chaining, and are not part of the canonical form or any CID (two hits are equal whatever text wrote them).
- **Gutter:** a quiet green dot on every line that is playing now (the current section's lines, and the call sites of what
  it expanded), a bright amber one on a line whose token just sounded, teal on a control line, red on one that is holding
  something changed (`for N bars`). A bar under the **section header** runs as far across as the song is through the section.
- **Control lines:** a ramp has a progress bar under its line (`filter lp 300 -> 2800` at 40%); a timed line (`at bar`,
  `before drop`, `every`) glows when it fires and fades over two beats, and so does the call site that made it.
- **Follow:** the editor scrolls with the song (windowed and full screen). When a section starts it eases (about 300 ms)
  so the section's block sits near the top with its header visible and its lines in view. A section taller than the view
  follows the lines firing now (a third of the way down). A def called from the section is brought in with it when both fit;
  when they cannot, the def's header pulses and an edge arrow names it. A ramp, timed line or def that fires off-screen gets a
  small arrow at the edge of the editor with its line number (`▼ 31`). Scrolling by hand pauses the follow for 8 seconds or
  until the next section; nothing scrolls while the text differs from the evaluated text. The **Follow** button or **L**
  toggles it, `windows follow window=live on=true|false` does too (no `on` flips it), and `btw get` carries `follow.on`.
- **Evolve:** the number (`gen 1`) sits at the end of the `evolve` line, and when a generation begins the block and the rules
  that ran flash purple.
- **The extensions' lines:** every new construct keeps its spans through `let`, `def`, `repeat`, `section` and `stack`
  (a hit that came out of a `stack`ed section lights its token in that section's text; one that came out of a wired
  value traces to the `$chords` in the consumer's body and to the call). A hit a chance dropped does not exist, so its
  token does not light. `duck`, `mix`, `sing` and a performed line are control lines: they glow when they fire and
  their `held` mark shows while a mix or a duck holds something changed. A `tempo` line is an event of the tempo map,
  not a control, so it has no progress bar (`btw get` says the tempo now). The new words (`stack`, `then`, `silence`,
  `wire`, `emit`, `assert`, `sing`, `duck`, `mix`, `lfo`, the waves and shapes, `knob` `pad` `fader` `held`, `when`)
  are keywords in the editor's colours.

Where it comes from: the engine publishes where each deck is (its beat) to the UI, not a stream of fired transitions, and
that is the clock the runtime already runs on. Each frame (about 60 a second while a program runs) the runtime takes the
bars the clock moved through and calls a hit fired when its step's time fell in them (`Live::watch_fires`), so nothing
touches the audio thread and a test can fire the same spans on a fake clock. (A true event ring out of the net runner would
be a change in `djrust`.) The markers are for the text that was evaluated: edit it and they wait for the next Cmd+Enter.

The windowed panel folds the change plan under one line, `plan: N changes` (collapsed, remembered), with a summary
(`+12 added · 3 changed · 1 removed`) and a scroll area when opened; `btw eval` / `btw get` keep the full plan (`changes`,
`plan`) and `plan_summary`.

`btw get` carries it as `marks`, for a reader that is not the window (an MCP client, the web `/p` page): `active_lines`,
`header` (the section header's span and the playhead's progress), `fires` (role, deck, `age_beats`, the token's `spans` and its
`call_sites`), `controls` (the line, its span and `via`, `age_beats`, `progress` of a ramp, `held`), `generation`,
`generation_flash_beats`, `evolve_spans`. Every span is `{start, end, line, col, len}` in the evaluated text.
Screenshots: `docs/layout-audit/btw/` (`cargo test -p djapp btw_markers_shots -- --ignored`, which plays
`examples/markers-demo.beatscript` on a fake clock).

## Evaluating and hot reload

`eval` parses, checks and compiles the text. Any error (a parse error, an unknown part or move, a range
that cannot be) means **nothing is applied**: the last good program goes on, and the errors come back with
line, column and a caret. A good program is *staged* and applied as follows:

1. It is **diffed** against what runs. A deck whose parts compile to the same net (same net CID) is not
   touched. A deck whose net changed is loaded with the new net. A control statement already running
   (same text) keeps its state: a ramp goes on, a `now` line is not run again, an `every` keeps its
   schedule. A new or changed statement is added. A removed `deck` line pauses that deck on the next bar;
   a removed control line stops (a ramp stays where it got to).
2. The apply happens in the first three beats of a bar (so the net is compiled well before the next
   line) and the engine installs it on the **next bar line** (`Engine::load_prepared`: "at the next bar
   when playing, at once when stopped"). When nothing plays it starts at once: the decks start. An
   evaluation sent in a bar's **last beat** is not applied then (there is no time to prepare it): it is
   applied in the next bar's first three beats and lands a bar later than "the next bar". The answer says
   which program bar it lands on (`lands`) and the window shows it.
3. A deck's new net starts at the same place in its alternation (`<a b>`) the old one was at, so a
   `kick` edit does not send a `<c4 e4>` melody back to `c4`.

`live check` runs 1 and 2's checks and answers what would change, without staging it.

Four extensions have a word in how an edit lands. A **tempo** line is not a control: when an edit gives the program
another tempo map (and one of the two is not a single number) the plan says "tempo curve changed", and the running map
is replaced as the edit lands, the runtime going on from the program's position. A **`mix`** or **`duck`** whose line
is dropped by an edit is put back at once (the old line is put back; the edited line waits for its section to come
round, as every `at bar` line does). A **`sing`** line's identity is the line and the notes it is sung on, so the same
text over an edited melody sings again and an edit of nothing does not. A **performed** line is a control like any
other (an edit keeps its state), and the take being logged follows the program: applying another program ends the take
(it is kept as the last one, and can still be saved). A line the edit keeps keeps its state, and what its section's end gave it
moves with the section: lengthen a section that has a running `mix` or `duck` (held to the section's end) or an `lfo`
(stopping there) and they go on to the new end, shorten it and they stop sooner.

## Canonical form

The **program CID** is the share CID (`z...`, sha-256, CIDv1, base58) of a document that names the version ("Versions"):

- BeatScript 2: `{"format":2,"text":<canonical text>,"type":"beatscript"}`, whose canonical text starts with the version
  line (`#! beatscript 2`, a newline, then the statements);
- BeatScript 1 (`#! btw 1`, or no version line): `{"format":1,"text":<canonical text>,"type":"djrust.live"}`, whose
  canonical text has no `#!` line at all, exactly as before the rename.

A canonical text says its version by its first line alone (`djlive::version::of_canonical`), so whatever holds one hashes it
right with nothing beside it. The canonical text is the parse, printed again, so formatting never forks a program. A text's **line ends** do not matter: `\n`, `\r\n` and a lone `\r` (an old Mac file) are the same
newline, and a UTF-8 **byte-order mark** at the start is white space; an error's line and column are the same whichever way
the lines end. These do not change the CID:

| normalised | rule |
|---|---|
| whitespace, tabs, blank lines, indentation | one statement a line, one space between tokens, blocks indented two spaces |
| line wrapping | a deck's `\|` continuation lines are one line |
| number spelling | shortest form: `18k` = `18000`, `0.50` = `0.5`, `50%` = `0.5`, `2.0` = `2` |
| note, key and chord spelling | lower case, flats as sharps: `Bb4` = `a#4`, `Bbm` = `a#m`, `BBMAJ7` = `a#maj7`, `D:Dorian` = `d:dorian` |
| order of params | sorted by name, in a part (`oct=2 vel=100`) and in a `do` line |
| comments | dropped: not in the CID, and not in the canonical text (the `#!` header lines are comments too: a name, the rights, `genre`, `tags`; the version line alone is kept, first, in BeatScript 2) |
| a scalar `let` | its value is put where it is used |
| an `import` | its snippet's CID replaces the name |
| a borrowed section or part | kept as written: `<cid>.drop` has the source's full CID in it |
| free numbers | a lone literal keeps the shortest spelling; arithmetic with no input in it is the number it comes to (`level 0.25 + 0.25` is `level 0.5`, `tempo 60 + 6` is `tempo 66`, `e(1 + 2, 8)` is `e(3, 8)`), a short decimal as that decimal and any other ratio as `n / d` (`1 / 3`); with an input in it the expression, its constant pieces folded (`level knob(1) * 0.7`, `when knob(1) > 0.5`) |
| counts, bars, lengths, degrees | arithmetic folds to the whole number it comes to: `x*(1 + 1)` is `x*2`, `section drop 4 * 4 bars` is `section drop 16 bars`, `at bar 2 + 2` is `at bar 4` |
| durations | `N bars` when whole (`1 bars`), else `N beats` when the beats are whole, else `P/Q bars` in lowest terms: `over 1/2 bar`, `over 4/8 bars` and `over 0.5 bars` are `over 2 beats` |
| positions | `bar W` when whole, else `bar W beat B`: `at bar 3/2` is `at bar 1 beat 2`, `at bar 8 beat 0` is `at bar 8`; after `from` a whole bar stays a bare number |
| a cycle, a euclid | `hihat/16 x` is `hihat x` (`/16` is the bar); `euclid(3, 8)` is `e(3, 8)`; a rotation prints as typed (`e(3, 8, 10)` is not `e(3, 8, 2)`) |
| chance | `?30%` and `?0.30` are `?0.3`; `?0` is a rest and `?100%` the bare term (`x?1` is `x`); a `seed=` prints only when the term has its own |
| degrees, numerals | `deg(1 b3 5' 1,)` one space apart with the marks glued; the letters of a quality in their canonical case, and the case of a numeral where the quality fixes it: `VII°` is `vii°`, `Idim7` is `i°7`, `iaug` is `Iaug`; a bare `voice-lead` is `voice-lead=on` |
| `stack`, `then`, `silence` | the laws of the structure: nested ones flatten, `silence` is dropped, a one-operand call is its operand, the operands of a `stack` are sorted by canonical text, a `then` is never reordered; `arrange then(a, then(b, c))` is `arrange a b c`; over parts `stack(...)` prints as the `\|` list sorted by text and `then(...)` as `<p q>`; a `let` of a lone name is the scalar `let` |
| def ports | `in: key, chords` is `in: chords, key` (sorted by type); a sole port is written `in` / `out` in a `wire` once the def is known |
| shapes, lfo | a ramp's `linear` is not printed (`over 8 bars linear` is `over 8 bars`); an lfo's params are sorted by name and `depth=1`, `center=0`, `phase=0` are left out |
| `duck`, `mix` | a duck prints both times, defaults filled in, in ms (`release=0.12s` is `release=120ms`) and `6.0db` is `6db`; a `mix` is one item a line, each number the value it comes to (`A level 0.5 + 0.2` is `A level 0.7`) |
| `sing` | the params sorted by name, the key spelled as everywhere |
| assertions | a clock as minutes and seconds (`90s` is `1m30s`, `500ms` is `0.5s`), a level expression with its constants folded |
| inputs | one spelling: `knob(3)`, `fader(A)`, `held(c#4)` (a note in lower case with sharps) |
| launch | `launch drop on pad(2)`; the default line (`next phrase`, `next phrase 8`) says nothing, `next bar` and `next phrase 16` / `32` are said |
| a guard | printed after its line: `... when $i % 4 == 3` |

**Doc comments.** The `#` lines written directly above a `section`, `let`, `def` or `arrange` line, with no blank line
between, are that item's doc comment (`djlive::doc`). The editor's hover shows it above what it says of the name, at the
definition and at every use in `arrange`; `check` gives each documented section's entry in `sections` a `doc`, and lists
them all in `docs` (`[{kind, name, doc}]`); a shared program's JSON-LD gives each documented section its `description`. A
leading `<name>:` that repeats the item's name is dropped (`# drop: the kick and the bass` above `section drop` is "the kick
and the bass"), several lines are one paragraph, the hover trims it to about 280 characters, and a `#!` header line is never
one. They are comments all the same: a program has one CID with them, without them, or with other ones.

What is *not* normalised, because it is meaning: the order of statements, the order of sections, a pattern's
steps (`x ~ x` is not `x x ~`), the tokens of a `def` or `repeat` body (a macro: printed as written, one statement a line, one spacing rule for all, so
`at bar $i + 1: master: level 0.5 when $i % 4==3` is a fixed point that is not pretty, and a `stack` in it is not sorted). Arithmetic is the number it comes to
(`tempo 60 + 6` and `tempo 66` are one program), but `e(3, 8, 10)` and `e(3, 8, 2)`, `emit c2`, `emit bass c2` and `emit out c2` play the
same and are different programs. Names are opaque to the laws of `stack` and `then` (`let both = stack(a, b)` is not the text
`stack(a, b)`).
`btw fmt` prints exactly this text and its CID; `share` encodes exactly this text. The editor's fmt, the MCP `fmt` (`text`) and
`beatscript fmt` print it with the `#!` header lines, comments and blank lines between blocks kept, and **upgrade**: a BeatScript 1
text comes back in BeatScript 2, its version line first, with the BeatScript 2 CID (`djlive::format::format_upgrading`; `from_cid`
is the CID of the text given, and formatting twice is formatting once). `beatscript fmt --keep-version` formats and keeps the
version. Programs sent by `eval`, `load`, `queue`, `share`, `push_private` arrive formatted with their version kept
(`djlive::format::format`: the same CID), so a text is never upgraded without being asked. The tests (`canonical.rs`,
`format.rs`) reformat the repo's programs at random (spacing, tabs, comments, `\|` line breaks, number and note spellings) and demand
the same CID, a changed note another. Every extension has such tests (reformatting keeps the CID, an edit changes it, the
canonical text is a fixed point), `examples_beatscript.rs` for `examples/beatscript/`.

## Saved programs

A saved program is a **licensed object**, like a `track` (docs/LICENSING.md): the canonical text and its rights
(CC0-1.0 public by default, CC-BY-4.0, all-rights-reserved; public or private; an author, `$DJ_AUTHOR` by
default), `forkedFrom` (the parent program's CID) and its credits. `btw fork` records the parent, names the
forker as the author, and carries the license and credits on; it adds the parent with its author as a credit
whatever its license (a `#! credit:` line marked `forked from`), so every author up the fork chain stays in the
lineage. A program that **imports** a CC BY snippet is credited to it, down the fork chain. Two CIDs: the
**program CID** (of the text) is what links, imports and `forkedFrom` name; the **object CID** (text and rights) names the file, so a new license is a new object of
the same program.

A `.beatscript` file in `<support>/live` (`$DJ_LIVE_DIR`) is the program with `#!` header lines (comments to the
language, all but the version line), so any `.beatscript` runs as it stands (and a `.btw` file is still read):

```text
#! beatscript 2
#! name: night bus
#! license: CC-BY-4.0
#! visibility: public
#! author: did:key:z6Mk...
#! forked-from: z4EBG9...
#! credit: z4EBG9... | did:key:z6Mk... | CC-BY-4.0 | the riffs
#! genre: https://beatscript.app/ontology/genre/techno
#! tags: https://beatscript.app/ontology/groove/four-on-floor, https://beatscript.app/ontology/instrument/acid
tempo 140
...
```

`btw export {id, path}` writes one anywhere on disk; `btw import {path | text}` and `btw load {path}` take one in
(its header's rights kept, a private one stays private). Older `<cid>.json` snippets are still read, as CC0
public. `track save` of a deck that plays a BeatScript program saves the program, not nothing.

The `genre` and `tags` lines link the program to the BeatScript ontology ("Links to the ontology"). They are read in any
spelling (`techno`, `genre:techno`, `bw:Techno`, an IRI) and kept as full IRIs, tags sorted and unique, genre first;
`btw save {genre, tags}` sets them; an unknown term is an error when the program is saved or imported (and a warning
in `check`). A change of genre or tags is a new object of the same program, as a new licence is; an object that links
to nothing is, byte for byte, the object it was before. `#! skip-test: <why>` is a header line for the examples
harness (the file needs a host to be tested): the store ignores it.

Beside each object `<object cid>.beatscript` the store writes `<object cid>.jsonld`, the program as linked data (`btw jsonld
{id}` makes the same text again); it is derived, so a failure to write it never fails a save. A **performance** (a
take of a program's performed inputs, "Performed inputs") is a third kind of object, `<cid>.perf.json`, with a CID of
its own: it names its program by the program CID, so changing a program's tags or licence does not detach it. `perform
{do: list}` lists them and `btw replay {id}` plays one back.

## Share links

```text
https://beatscript.app/#<CID>.<base64url(deflate(formatted text))>[.<sig>][*<base64url(deflate(programs))>]
https://beatscript.app/#<CID>.<base64url(deflate(canonical text))>[.<sig>][~<base64url(deflate(notes))>][*<base64url(deflate(programs))>]
```

`btw share` makes one: the sealed part is the CID and the program's **formatted** text (what `fmt` prints: the
canonical text with the `#!` header lines, the comments and one blank line between blocks kept), so a program
opens as it was written. The CID is still the canonical text's: the header lines and comments are in neither the
CID nor the signature, and come along **unsealed** (open says so). `sig` is the author's wallet signature over the
CID, carried as given (the wallet makes and checks it). The site's links are on `https://beatscript.app` unless
another origin is named. BeatScript links are made on beatscript.app only: a link made on an older host
(`btw.bitwrap.io`, `beats.bitwrap.io/p#`) still opens, since only the part after `#` is read, and none is made there. The
engine's form (second line, what the apps and the booth make) packs the canonical text; there `~notes` carries the
comments, shown as **unverified**. `btw open <url or CID>` decodes it, parses and canonicalises the text, recompiles it and
recomputes the CID, and **refuses** a link whose CID is not its text's (`refused: the link says z... but its text
is z...`). It answers the canonical text (what is stored and hashed) and the text formatted, with what the link
carried beside the program, for the editor. A link made before links carried the formatted text packs the
canonical text and opens as it always did; a text that is neither canonical nor formatted is a warning (the CID is
still its canonical text's); a CID alone is looked up in storage, where there is any.
**A program that draws on others carries them.** For an `import` or a set's tracks, the part after the `*` holds the canonical text of
each program it draws on, and of each of those in turn, by CID (what a program needs first). Each is hashed against its CID before
it is used, and a text that is not the program its CID names is refused (`the program z... in this link is not the program it names`),
so a link opens on a machine that has none of them, and whether it checks never depends on the machine. The `*` part is not sealed (the
CID is of the program alone) and is not signed. A link is as long as that makes it (over 2000 characters it says so); the programs
together may be 512 KiB, and past that no link is made. A program that imports one that is saved **private** is not shared, since the
link would carry its text.
A private program is never shared. A link over 2000 characters says so. The link seals the canonical text only: the `genre` and `tags`
lines and a program's performances are not in it (a link is the program; its links to the ontology are its author's statement about
it, kept with the saved object, as the licence is). Tonight's programs:

| program | canonical text | link |
|---|---|---|
| hand-dubstep-140 | 778 bytes | 405 characters |
| hand-proghouse-124 | 1236 bytes | 545 characters |
| acid-rave-138 | 1256 bytes | 498 characters |
| hand-dubstep-140-v2 (64 bars, six sections) | 1863 bytes | 819 characters |

## Errors

```text
error: expected a part's pattern, found `|`
  --> 3:14
   |
 3 | deck A: kick | snare ~ x
   |              ^ a part is `role pattern`, and this one has no pattern
   = expected: `x`, `~`, a note like c3, a degree like 3, `[`, `<` or a generator like e(5,8)
   = help: kick x ~ x ~
```

An extension's errors are of the same kind. A wire that joins two ports whose types do not fit:

```text
error: type error: `groove.out` is a drums pattern, `walker.chords` wants chords
  --> 7:6
   |
 7 | wire groove.out -> walker.chords
   |      ^^^^^^^^^^^^^^^^^^^^^^^^^^^
   = help: a `chords` port carries chord symbols like am7 and fmaj7, Roman numerals like V7, or chord(am7) calls, as a pattern
```

Every diagnostic has a severity (`error`; `warning`, for what may be wrong; `info`, for what the check found out and has
nothing wrong to say about: a chance's period, a polymeter's period, the ontology's name for a rhythm, a pending render
assertion), a byte span, a line and column, a message, the
list of what was expected, what was found, and an optional help line. The caret form above is
`render`; `live eval` and `live check` return the same as JSON (`diagnostics`) with `line`, `col`,
`end_col`, `message`, `expected`, `found`, `help`.

## The `btw` noun

| verb | does |
|---|---|
| `eval {text, name?}` | parse, check, compile and stage: lands on the next bar. On an error nothing changes and `diagnostics` says why; `ok` false. Answers `ok`, `cid`, `nets` (deck, or `section@bar:deck`, to net CID), `changes` (what the diff will do), `lands`, `warnings`, and the status; where the program has them also `length` (through the tempo curve), `mix` (the registry calls of its `duck` and `mix` lines) and `lineage` (an evolve's chain) |
| `check {text}` | the same without staging |
| `get` | the running program (canonical text, CID, since which bar), the section and the bar in it, what is staged, the last errors, `warnings` (a muted strip a program plays on, a tempo the host cannot set, a performed input nothing answers for), the decks it plays on, the tempo now (`tempo`, `tempo_curve`) and `performed` (the inputs it reads, whether it logs, the take's CID) |
| `fmt {text?}` | the canonical form, its CID and the comments |
| `save {name?, text?, license?, visibility?, author?, genre?, tags?}` | keep the running program (or `text`) as a licensed `.beatscript` object, with its links to the ontology; answers the CIDs, rights and warnings |
| `load {id \| path, eval?}` | a saved program (CID, part of one, name) or a `.beatscript` file; `eval: true` (the default) stages it |
| `list` | the saved programs, with their rights, `genre` and `tags` |
| `fork {id, name?, author?}` | a fork: `forkedFrom`, license, credits and links carried |
| `license` / `visibility {id, value?}` | read or change them (a new object of the same program) |
| `export {id, path}` / `import {path \| text}` | `.beatscript` files on disk |
| `share {id \| text, sig?, notes?}` | the link |
| `open {url, eval?}` | decode, recompile, verify; into the window, staged with `eval` |
| `generation {gen, scope?, text?}` | the CID of generation `gen` of an `evolve`, made from generation 0 and the rule alone (no chain kept), with the evolve's tail and period |
| `instruments` | the voices, family words, kits and palettes |
| `jsonld {id? , text?}` | the program as JSON-LD: its CID, genre, tags, key, tempo, sections, rights, forkedFrom, credits and Euclidean rhythms |
| `terms {kind?}` | the vocabulary the `#! genre:` and `#! tags:` lines are checked against: kind, local name, IRI and label of each term, and where the cached copy is from |
| `test {text? \| id?, sample_rate?}` | judge the program's `assert` statements: the static ones re-checked, the `level` ones measured on an offline render; each reported pass or fail with the measured values, and `ok` false when one fails |
| `perform {do?}` | the performed inputs of the running program and the performance it has logged: `status` (default), `start` (a new take), `stop` (stop logging), `save` (keep the take as a performance object), `list` |
| `replay {id}` | play a saved performance back: its program is evaluated afresh and its inputs read the performance's values, by bar, instead of the controllers |
| `stop` | stop the program (the decks play on; the controls and ramps end; a strip a move muted is put back; a `duck`, a `mix` and a performed line's holds are put back) |

From a shell, with no app: `djctl btw fmt | check | share | open | render | test | nets | gen | save | instruments |
jsonld | terms | replay`. `check` prints errors and warnings with line and column (exit 1 on an error), the length of
the song through its tempo, and the notes the extensions make (the period, the chances, the performed inputs, the
rhythms the ontology names); `test FILE [--sr N] [--json]` exits 1 when an assertion fails; `gen FILE N` is
`generation`; `nets FILE [--sections]` prints the nets as CIDs (a song longer than 64 bars is chained nets with no single
CID: it says so, and `--sections` lists each section's; a song with no longhand form, because a part differs between
sections, still lists its sections with `--sections`); `replay FILE|CID [OUT.wav] [--program FILE] [--bars N] [--sr N]
[--beats N]` plays a performance offline and prints the calls its control lines make (a value past the program's end is
refused). `djctl btw render FILE OUT.wav [--sr N] [--beats N]` plays a song's sections, nets and sounds offline
(docs/HEADLESS.md). `--sr` is 8000 to 192000 and `--beats` is 0 to 16384 (one number is read the same by `test`, `render`
and `replay`, and a word that is no number is an error); `--bars` is 1 to 16384.

The window's title is `btw`: the editor, **▸ section n/m** beside the bar and phrase, errors with carets, what
will change, the CID and a name field, the saved programs. A deck that plays a program shows `btw: <name>`
and its section, bar and phrase where a track shows its genre and seed.

### The offline render

`djctl btw render` and `btw test` play a program without the app: `djhost::beatscript_render` turns the compiled song into an
arrangement (`djrust.arrangement/1`, which `djapp_render` also takes) and the offline renderer, the engine djapp plays
on, plays it. **In it:** each section's nets, loaded a bar before the section so they land on its bar line, and a deck
a section does not use stopped; a **tempo curve**, as a staircase of tempo-fader moves whose seconds add up exactly to
the integral (the fader reaches ±50 % of a net's own tempo, the engine's limit: a curve in one net that goes further
is held at the edge and `djctl btw render` prints a `note:`; sections make each net at the tempo it begins at);
**`duck`** (a `trick` live op: the engine's pump, which hears deck A's kick) and **`mix`** (`param` and `master`
actions, a section's snapshot put back where the section ends; with no mixer to read, what it goes back to is the
engine's starting positions), each placed on the exact beat it falls on and tagged with the tempo of the step it falls
in, `every` lines expanded over the render's length (at most 4096 firings of one line); and, with `djctl btw replay
... OUT.wav`, the **mixer calls** of a performance (a strip's volume, EQ and mute, the master level, the crossfader).
**Not in it:** every other control line, because the arrangement format has an op for those and for nothing else of
the registry: a filter, an insert, a `do` call, a ramp or an lfo, a `move`, a `load`, a `queue` and `sing`. So a
render has the song's notes, sounds, sections, tempo and mix, not its riser sweeps, and a level assertion about a
filter sweep measures a render without it. A program that has no end renders 16 bars unless `--beats` says; a
performed program renders with every input at 1/2.

`beatscript render` and `beatscript listen` (the site's render) play more: the program's control net (its inserts and ramps, and
`filter_exit` and `echo_out`, which the net holds), and the other **moves**. Those are armed on the booth's own move
scheduler (docs/PERFORM.md), on the render's clock and never the wall clock: a `loop_roll` (on a net deck, the beat
repeat), a `bass_swap`, a `phrase_sync` with `do=cut`, a move with `for N bars`. Each lands as it does in the booth, on
the next bar or phrase line after its line (the song's lines, counted from its first bar), and a strip it muted is put
back on the first phrase line after it. Two moves cannot act in a render, and its notes say why: `vocoder_handoff` (a
render has no vocoder) and a `phrase_sync` that steps a seed (a program's deck plays its own nets, not a seed).

## Tests

`crates/djlive` has 122 unit tests (in the modules: `rat`, `expr`, `dur`, `lex`, `scale`, `meter`, `chance`,
`algebra`, `ports`, `mixmoves`, `sing`, `tempo`, `shape`, `assertion`, `perform`, `linked`, `music`, `highlight`) and
763 integration tests in 32 files (counted at commit 192a4b4):

| file | tests | what it holds |
|---|---|---|
| `tests/beatscript.rs`, `language.rs` | 37, 42 | the base language: parser goldens, lets, defs, `repeat`, sections and `arrange`, `after`/`before`, `for N bars`, `do`, sugar, ramps, instruments, transforms, imports, determinism, the expansion cap; errors with line, column and message |
| `tests/beatscript_runtime.rs`, `hot_reload.rs`, `marks.rs` | 13, 16, 10 | a song on a fake clock (nets land on bar lines, controls count from their section, a move puts its strip back, `stop`), hot reload and diff, the editor's marks |
| `tests/canonical.rs`, `evolve.rs`, `trance.rs` | 8, 13, 4 | random reformatting keeps the CID, links seal their text; evolve; the 64-bar song equal to its longhand |
| `tests/examples_beatscript.rs` | 2 | every `examples/beatscript/*.beatscript`: compiles, canonical text is a fixed point, 40 reformattings keep the CID |
| `tests/ext_arith.rs`, `ext_timing.rs`, `ext_tempo.rs`, `ext_shape.rs` | 21, 31, 22, 27 | arithmetic and guards; sub-bar times on a fake clock; the tempo map (a law: `bar_at(seconds_at(b)) == b`, the seconds add up) and the runtime; shapes against hand formulas and lfos |
| `tests/ext_scale.rs`, `ext_meter.rs`, `ext_chance.rs` | 47, 31, 34 | degrees, numerals, voice leading (checked against an exhaustive search on 400 random cases); `e()` and `role/N` (periods, the ontology names); chance (golden hash values, the empirical rate over 4096 bars) |
| `tests/ext_algebra.rs`, `ext_ports.rs` | 50, 52 | the laws of `stack` and `then` (160 random compositions: re-association, unit insertion, operand permutation keep the CID; the song equals an independent oracle's longhand); the wiring laws (bracketing, order, identity, loops against a peeling oracle, 300 random graphs) |
| `tests/ext_mix.rs`, `ext_sing.rs`, `ext_assert.rs`, `ext_assert_cross.rs` | 27, 27, 31, 14 | `duck` and `mix` (a snapshot is put back whatever the song: 80 random songs); `sing` on fake voices; assertions and the length through the tempo map |
| `tests/ext_perform.rs`, `ext_perform_cross.rs`, `ext_linked.rs` | 45, 21, 32 | performed inputs (a live run with scripted controllers against the replay of its log, call for call); links and JSON-LD |
| `tests/ext_cross.rs` ... `ext_cross7.rs` | 13, 15, 18, 18, 13, 12, 17 | features that were built apart, joined, each test naming what it joins: the first two files arithmetic, guards and sub-bar times with ports, polymeter, chance and imports; then degrees and numerals with the rest; `stack` with keys, chance, cycles and wires; `mix` with arithmetic, sub-bar times and stacks; `sing` with defs, imports, stacks and chance; tempo curves and shapes with stacks, mixes and ports |

`crates/djhost`: unit tests in `beatscript_env` (every verb of every noun reachable through `do`, the family words pinned to
the engine's own, the voices), `live_store` (rights, forks, import credits, `.beatscript` round trip, legacy files, links,
the JSON-LD pair, performances), `beatscript_render`, `beatscript_test`, `beatscript_inputs`, `ontology`, `vocals`, and the integration
tests `tests/ext_linked.rs` (14), `beatscript_tempo.rs` (9: a kick on every beat lands within 12 ms of the second the map
gives its beat; a duck heard under a curve) and `beatscript_test_tempo.rs` (5). `crates/djctl`: `tests/btw_test_cmd.rs` (7),
`btw_replay.rs` (4), `ext_linked.rs` (4). `crates/djapp` `app_tests`: the registry round trip over every `btw` verb,
the window's buttons, and the verbs on the real engine (fmt, instruments, share, open refusing a forgery, fork,
export, the deck panel's line, `track save` of a program deck; a duck on the real pump; a `for` mix reading the real
mixer; `btw test` measuring levels; performed inputs through the bindings; a performance kept and replayed).

## Decisions

The readings of the points the contract left open, each with its reason. Where a long note holds more, it is named.
(The orchestrator's log of how the features met is the end of `docs/BEATSCRIPT-EXTENSIONS.md`, section 5; the workers' own
logs are the "Decisions" of each `docs/beatscript/ext-*.md`.)

**Names and reserved words (the syntax review, SYN-01)**

- A word of the extensions is reserved only where it starts a construct that a name could not be told from:
  `stack then silence wire assert sing under choose maybe deg lfo and or not voice-lead s-curve emit` (`parse::EXT_RESERVED`; a
  *def* may still be called `stack` or `then`: at the start of a line, with no part inside, `stack(A)` calls it).
  Every other word the extensions use (`pad drums depth test genre tags replay length period step in out chords pattern
  rhythm center phase release attack eq exp log sine tri square saw linear beat beats weights ends never beside during knob fader held min max mix duck`
  and the Roman numerals) names a `let`, a `def` or a section as it did before the extensions, and is read as syntax by its position.
  *Old programs use `pad`, `drums`, `depth`; reserving 107 words everywhere stopped five of the author's saved tracks from reading.
  Every saved program the old build checks gives the same canonical text and CID with this build (`tools/diffold.py`).*
- The positions: a function before `(` (`min(`, `pad(5)`, `knob(3)`, `held(key)`; `duck(` and `mix(` call a def of that name);
  a wave after `lfo`; a shape (`exp log step linear`) after a duration (`over 4 bars exp`); `beat` and `beats` after a number;
  `length`, `period`, `ends` and `never` right after `assert` (and `ends` before `on`, `beside` and `during` after `assert never`),
  and `in` anywhere in an `assert`; the port types and `in` / `out` in a
  def's header and after `emit`; `eq` and `pan` after a deck in a `mix`; `weights` after a choice's group; `mix` before `{`; `duck`
  before a deck; a name after a dot (`a.out`, `A.kick`); `depth= center= phase= attack= release=` before `=`.
- A scalar `let` (`let n = 4`) is put in place of its name wherever the name is read as a name, and **not** where the word is read
  as one of those positions, and never in a def's header (a def's parameter may be called as a let is: `def grv(deck, n)`). A scalar
  `let` of a number and a unit (`let atk = 5ms`) is put in place as both tokens. A scalar `let` that has the name of a def or a
  section is an error that names the let (`` `drop` is the let on line 1 (= 8): a section needs a name of its own ``).
- `control` is no reserved word: it is the control deck's head only before a colon, at the head of a line or after a time word's
  colon (`parse::control_head_at`), so a let, a def or a section may still be called `control` (`after control: control: queue
  next`, `control(A)` calls a def). *No program in the repo names anything `control`, but a reservation would buy nothing.*
- A Roman numeral that names a `let` wins in a pattern: `let v = c2 e2` then `bass v` is the let. *The author's own `i`, `v`,
  `step` are names; `V` in upper case is still the numeral.*
- A name that cannot be used says what to write instead (`rename it, for example `pad1``); a `a`..`g` name (a note) too.

**Marks and comments (the syntax review)**

- A `#` glued to a numeral's letters is a comment unless the line is a pitched part, a `let` or `emit` of numerals, or the arguments
  of a call, **and** a step, a mark or a param follows it. *`kick x ~ x ~ #v` was a comment before the numerals and still is;
  `harmony I V #I like it` is a comment too. A trailing `#v` after a pitched part is still a sharp numeral (the one case that
  cannot be told).* A `#` inside a `deg( )` that is no sharp is an error, not a comment that eats the rest of the line.
- Inside a `[ ]` group of `deg( )` a comma is the stack comma however it is spaced (`deg([1, 3, 5])` is the chord); an octave-down
  mark there is the comma before `]` or another comma (`deg([3 1,])`). At the top of the `deg( )` the comma followed by a space is
  still the octave mark (`deg(1, 3, 5)` is three degrees an octave down, documented).
- A `when` guard guards the whole line, also after a line of several parts (`deck A: kick x | snare x when ...`): a warning says
  so and how to guard one part (its own line).
- Newline rules are not uniform and are left as they were: `arrange stack(` and a part-level `stack(` / `then(` may run over lines;
  `deg(`, `e(`, `maybe(`, `[`, `<`, a def's call and its parameters, and a `{` on the next line (a section, a def, a mix) may not.
- The end of the text is where the last token ends: an error there is on the line that stopped short, with blank lines or comments
  after it.
- After an error the parser skips the rest of the statement and of its block, so a header that did not read does not flood the
  report with its body; a section may close on the line of its last statement (`section a 4 bars { deck A: kick x }`).
- A diagnostic found in the body of a def or a repeat is said once, however many calls run it, and names the (first) call.

**Arithmetic and guards**

- Arithmetic is exact: rationals, `/` exact, evaluated once when the program is read. *Total and deterministic; bars,
  positions and seconds never drift.*
- `let r = 3*2` is the number 6, not the degree 3 twice. *The one deliberate change of old meaning; no fixture used
  it, and `let r = [3 3]` says the old thing.*
- A lone literal is read as it always was, not through a rational. *`2k`, `50%` and `0.123456789` keep their spelling
  and value, so no existing CID moves.*
- A number with more than six decimals is an error inside an expression and kept as typed when alone. *A rational
  needs an exact decimal.*
- `%` is modulo only with a space (`$i % 4`). *`4%` is the percent mark.*
- A def's arithmetic argument is put in place in parentheses where the slot needs them, except a plain ratio (`4/2`)
  that is not in a glued slot. *A macro must not change the grouping; a `do` value reads a plain ratio as a word.* The
  slot decides: **glued** (`x*$k`, `x?$p`, `hihat/$n`, `weights $w 1 1`) takes every arithmetic argument in
  parentheses, a ratio and a `min(...)` / `max(...)` call too (`gt(A, 8 / 2)` is `x*(8 / 2)`); **beside an operator**
  (`$a * 2`) as before; **a step of a pattern** (`bass $p ~ 5`) takes one in parentheses unless it is only numbers and
  `*` (`1*4` is the degree 1 four times, `1*2 3` two terms: what they meant before arithmetic, so no old program's
  meaning or CID moved; `(1 * 4)` would have been the degree 4 once). Anything else (a sum, a call) was never a
  pattern, so it is a degree computed there (`pulse(1 + 2)` plays degree 3). *The reviewers' finding L-04.*
- `$i` and `$n` of a `repeat` inside a def's body are the repeat's: the def's own fill leaves them for it, and they are
  never offered as the name of a misspelt parameter. *A `repeat $bars { at bar $i + 1: ... }` in a def is the first
  thing `$i` invites.*
- A percent mark glued to a digit (`7%2`) is an error that gives the modulo spelling (`7 % 2`), not two numbers.
  *`7%` is seven hundredths, and the two tokens failed far from the cause.*
- A bound is a count of what is read, never a clock. *Total and deterministic: the same text is the same error on any
  machine.* The numbers are in "Bounds on the text"; they are far above what a song uses (the reviewers' attempts to
  cross them were nested repeats of 64, `*64` inside `*64`, chains of 800 terms and lets that double).
- A block whose head did not read (`section intro 0 bars {`, `repeat 0 {`, `def go(a b) {`) is skipped to its closing
  brace, so one mistake is one error and the lines after the block are still read. A `def`, `let` or `section` whose
  name was refused is said once, at its definition, not again at each use.
- A guard is evaluated where the line is expanded, and a false one removes the line before it is checked. *So one
  `repeat` can name things that exist in some copies only.*
- Guards belong to lines, not to `tempo`, `key`, `let`, `def`, `section` or an `evolve` rule, and an `emit` takes
  none. *Those have no line to drop; a value that is sometimes absent has no type.*
- Slots glued to what comes before them take parenthesised arithmetic (`hihat/(4 + 8)`, `x?(1 / 4)`, `weights (1 + 1)
  1 1`). *A def's arithmetic argument arrives in parentheses there and must fit where `$n` stands (even a ratio).*
- Tokens that must touch (`x?0.3`, `hihat/12`, `A.kick`, `6db`, `==`, `5'`) stay touching when a macro puts a value in
  place or an import re-spans a def (`lex::put_at`, `lex::touches`, `glued_pair` in the tests). *Found by joining
  ports with chance and polymeter.*
- A comment starts with `# `; `#iv` is a sharp numeral. *The lexer cannot tell a comment of words from a numeral, so
  the error that follows says so.*

**Time and tempo**

- A program with no sections keeps counting its bars from the bar it was evaluated in. *The review found (R-02) that a
  program with no sections, evaluated while a deck played, counted its bar 0 on a bar nothing of it could sound on while the
  offline render, a song and the tempo map counted the first bar you hear; one reading would have made the runtime agree
  with them and moved every timed line of every program written for the old count one bar later live. The author performs
  live with those programs, so the count was not changed: `at bar 1` is still the first bar you hear. What is counted from the
  first bar you hear is what has no old program: a tempo curve (and a `tempo` line) of a program with no sections starts on
  the bar the music lands on, live as in the render. The cost, written under "Time words" and in `ext-render.md`, is that the
  render of a flat program has its timed lines one bar later than they fire live when a deck was already playing.*
- A chained program (too long for one net) goes round on its own bar line, once a lap, and a song that loops loads its
  first net in the bar before the lap turns. *The chain's lap turned a bar early and then at every bar line; a looped song's
  first net came a bar late once the first lap had turned. Neither depends on how the bars are counted.*
- Beat offsets are 0-based like bars (`at bar 16 beat 3` is the last quarter; `beat 4` is an error). *Consistent with
  `at bar 0`; `at bar 17` is the next line.*
- Whole-bar words keep their old tree and print as before. *No CID moves.*
- A time between bar lines is exact in bars and compared with the runtime's position with a nanobar of slack; an
  `every` finer than 1/64 bar is an error; at most 32 firings of a line in a pass. *The runtime looks about sixty
  times a second.*
- A def's call takes a whole bar, and sections, generations and `repeat ... every` are whole bars. *A finer base would
  put a fractional base in every body line; no program needs it.*
- A tempo ramp is linear in beats. *The bar clock never moves; seconds are an integral in closed form.*
- A tempo is an event of the tempo map, not a control: no registry call, no key, nothing to put back, and it happens
  once at a place (no `every`, no `for`). *A map is a function of the bar built at compile time.*
- A program whose tempo is one number is not driven and keeps its nets and CIDs; a program with a curve makes each
  section's nets at the tempo the curve has where the section begins. *Nothing written before F3 moves.*
- Before its first tempo a program says nothing; a length counts 120 there and says so.
- Two sections that play at the same time and set different tempos (or keys) on one bar are an error, not a
  resolution. *A net has one tempo and one key.*
- No `lfo` on a tempo. *No closed-form integral and no use the curves do not cover.*

**Shapes, lfo, mix**

- An lfo is compiled through a ramp from `center - depth` to `center + depth`. *The range checks and every param's
  unit are the ramp's, once.*
- The waves start on the midline (a square starts at the top). *A filter that starts at its centre does not jump when the lfo begins.*
- `depth=1`, `center=0`, `phase=0` are left out of the canonical text. *Two spellings of one lfo are one program.*
- An lfo out of range is held at the edge with a warning, not an error. *The contract says clamped.*
- `step N` is 2 to 1024. *A ramp sends one value a step; one step has nothing to step to.*
- A mix item is one number set once: no shape, lfo, ramp or input. *A ramp is a `level` line already, and the error names it.*
- A `do` word with a dot (`param=delay.mix`) reads as one word again. *The lexer had cut at the dot; found by the
  shapes and the performed tier.*
- A `duck` is the engine's own pump (`trick` kind `pump`), not a new gain stage. *Sample-accurate and independent of
  the fader; the cost is that only a low part (`kick`, with `bass` warned) keys it, the attack is the pump's 2 ms and
  the release is a note.*
- A duck prints both times and a mix the numbers it comes to. *One meaning, one spelling, one CID.*
- "Restored on the section's last bar line" means the line that ends the section; a mix in a section fires once; at a
  bar line every put-back runs before any line fires. *The section's last bar is mixed as the rest is; a line written
  on that bar line has the last word.*
- `A pan` is read and refused. *The mixer has no pan on a strip.*
- A mix in a stack is held to where its own section ends, and two mixes holding one knob at once are an error. *The
  one that ends first would put back the other's value and leave the mixer changed.*

**Degrees, meter, chance**

- Degrees are of the key's own scale and accidentals are relative to that scale's degree: `b3` in A minor is B. *The
  contract's reading; `#7` is the leading tone.*
- A bare `b3` outside `deg( )` stays the note B3. *It always was.*
- A numeral is the chord on a degree of the key's scale, and its case is the chord's colour, not derived from the
  scale (`VI` in A minor is F major). *Case is how a person writes a chord.*
- `I/3` is the first inversion, and a single-note role plays the bass tone. *With a slash the slash must do something.*
- `arp` plays four steps over its numeral's slot. *They land on the sixteenth grid for 1, 2 or 4 chords in a bar.*
- The case of a quality that fixes it is normalised (`VII°` is `vii°`, `iaug` is `Iaug`). *Spellings of one chord are one program.*
- Voice leading is greedy, chord by chord in reading order, and a one-bar part's loop point is not led.
  *Deterministic, with ties to the lower inversion and then the lower octave.*
- `1,` is the octave-down mark only directly inside `deg(`, and its boundary also admits `>`, `*`, `!`, `^`. *So a
  down-octave degree can carry a repeat, an accent or a slide; `[1,3,5]` stays a chord.*
- A section's `key` retags a net only for a deck that plays pitches. *A deck of drums keeps its net in every key and is not reloaded.*
- The period error is for parts with a cycle of their own, per deck; the report is per program. *A long alternation is
  chained as it always was; a net is a deck's.*
- An alternation inside a `/N` part advances once a cycle of that part. *The cycle is the part's bar.*
- A rhythm is named for its own role, and no folk name is invented. *The ontology's individuals are role-bound and has
  no "tresillo"; an invented IRI would be a lie.*
- A chance is decided by a hash of (seed, bar, cycle, step, note), not the contract's four. *Without the place two
  `x?0.5` in one part would be one toss; without the cycle the two repeats of `fast` would.*
- A program with no sections unrolls its chance over 16 bars, not the contract's 64. *A phrase and a quarter of a net;
  a section says more.*
- A probability or a weight has at most six decimals; chance is per event (a chord is thinned note by note); weights
  are read by count. *Exact rationals; a whole chord is a `choose` or a `maybe` on one note; a degree step after the
  weights cannot be told from a weight.*
- The hash is SplitMix64's finalizer chained, pinned by golden values. *FNV over small integers mixes the low bits poorly.*

**Composition and ports**

- The laws are applied when the expression is read, and a plain list stays `Arrange`. *One tree for one program;
  existing programs go through unchanged code.*
- A stack is cut into consecutive stretches, not overlapping segments. *The runtime, diff, render, marks and `btw get`
  needed no change.*
- `stack` over parts is sorted only where it is spelled `stack(...)`; a `|` list as written is not. *A `|` list is
  existing text whose order is in its CID.*
- Names are opaque to the laws (`let both = stack(a, b)` is not `stack(a, b)`); a `let` of a lone name is the scalar
  `let`. *A `let` is a statement of the program.*
- `then` over parts is the alternation `<p q>` and needs one role, one parameter set and one cycle. *The result is one part.*
- Controls of a padded section are not clipped at its end; a section with an `evolve` cannot be stacked. *An `every`
  line behaves as in a plain section; an unequal stack must not silently change what an evolve means.*
- A later stretch unrolls its chance over `phase + bars` and sings over the whole section once. *It goes on from where
  the section was.*
- Wires join defs, not calls; a def reached only through a wire is not played. *"Each input wired once" is per def, so
  a def is a module and a call plays its lines.*
- An `emit` may not use the def's parameters. *An out value is then a function of the wiring, which the laws need.*
- Pattern and chord values go in as one group, and bare chord symbols become `chord(x)`. *`$in` is a unit wherever it
  stands; a progression plays in `harmony` with no ceremony.*
- A `rhythm` fits `bass`, `melody` and `drums` inputs, and an unwired input is an error at a call, not at the def. *A
  rhythm is hits and rests, which every part reads; a library of modules has inputs nobody has wired yet.*
- A snippet's wires come with its defs. *Without them a saved composition would lose its wiring.*

**Sung lyrics, assertions, performed tier, links**

- There is no neural voice in this repo, so the mechanism is built against fakes and the real host says "not
  installed" and plays on `robot`. *The only neural thing in `feat/vocal-neural` is the beat detector.*
- The vowel-group syllabifier is the language's and the dictionary's count wins where the host knows it.
  *Deterministic without a host, and no word drifts from what the voice sings.*
- A line's cover is `bars=`, else the section, else the part's own cycle, and a note lasts to the next less a
  sixteenth. *A sung vowel needs the time.*
- `for N bars` on a `sing` is refused. *The clip id the loop row gives is only known when the line fires.*
- A line's identity includes a hash of its notes. *An edited melody sings again on a hot reload.*
- An assertion is a top-level statement and statics are errors when false; a pending level assertion is a note (`info`).
  *`in SECTION` says where it is about; `eval` refuses a false static.*
- A constraint (`never X beside Y`, `never X during Y`, `X ends on D`) is an assertion about the notes, decided by `check` over the whole
  finite timeline and refused when false; a part with chance is judged on everything it may play, and the degree is 1 to 7. *Enumeration is
  exact where a pflow invariant says nothing about which transition carries a snare; the strict reading of chance is the one that holds
  whatever the text rolls; a numeral's case is a chord's quality, so a degree is a number.*
- A program with no end has no length, and a tolerance goes only with `==` and `!=`. *A length is not guessed from one
  lap; on `<` a tolerance would be ambiguous.*
- Levels are linear RMS of the offline render's stems, not dBFS, and a level assertion must read a deck. *The
  contract's own numbers are linear; `level 1 < 2` would be a tautology.*
- `period(NAME)` is an evolve's only; `period` alone is the least common multiple over the nets. *The contract's
  reading of "as meter reports it".*
- A control with an input is compiled once as if every input read 1/2, and its range is judged from interval bounds.
  *`level 3 * knob(1)` must not be refused for being 1.5 at one half.*
- Inputs are read where a line is due (a bar line, or the 1/64 of the bar it fires at), once for lines due together, and logged then.
  *The log is the values the program used, by bar and place in it, and the replay answers exactly those reads.*
- A value line with no time word follows its input, and a knob nothing answers for rests at 1/2. *A knob is a thing the hand
  holds: a line that read it once left the rest of the song where the first bar put it, and a program shared to a page with no
  knobs started with the filter at 200 Hz. An old take of such a line has one reading, and replays its later bars at 1/2.*
- A launch is a statement with a pad, not a binding or a knob statement. *A pad is a role (`pad(2)`), as `knob(3)` is; which device has it is the booth's, so no device or controller number is in the program or its CID, and a program shared to the page checks the same.*
- A scene is a section or a named arrangement, a section scene loops. *One line each, no name clash (they share a namespace), and `check` already warned that an arrangement no `start` reaches never plays: a launch is how it does.*
- Each scene is compiled once more as a program of its own. *The runtime swaps in a whole timeline, controls and tempo curve and all, at a bar line, as it does at a lap; the cost is one compile for each scene a program names, and the written song is as it was.*
- An input written alone as a cutoff spans the filter's range. *`filter lp knob(1)` is the line the knob is for; the range error
  said to scale it by hand.*
- Replay lives in the runtime, not in a host wrapper. *A host that forgot to answer from the log could not make a replay differ.*
- An input in an `evolve` rule's number is an error. *Generation N is made from generation 0 and the rule alone; a
  value read from a controller would make its CID depend on a performance.*
- A def call takes no performed guard and a `let` cannot hold an input; a port carries a number fixed when the program
  is read. *Each is an error with the way out, not a rule that reads the input at expansion.*
- `btw test` and the offline render read every input as 1/2 and say so. *The render has no controllers.*
- The links store the published term IRIs (`.../genre/techno`), and accept the contract's `bw:Techno` and the
  namespace IRI `https://beatscript.app/ontology/ns#Techno` as spellings. *The published ontology keeps its classes in `ns#` and its terms at `/<kind>/<slug>`.*
- A bad term is a warning in `check` and an error in `save` and `import`. *A header line is a comment and must not
  stop a live program; a typo kept in a saved object persists.*
- A Euclidean node of the JSON-LD has a `name` only when the ontology names the shape once, and `sections` are the
  song's sections as placed, not the stretches. *Six genres share E(4,16), so a name would be arbitrary; a stack's
  stretches are the compiler's cut.*

- **The second review's decisions (fixer b).** A chance part's implicit seed is of the deck, the role and the pattern, not of its
  params (a part with no chance keeps the text it hashed, so no old net moves). `x?0` is a rest and `x?1` the bare term. A chance net
  has no period. A time inside bar 0 of a flat program is legal and only the whole bar 0 is refused. Arithmetic with no input folds to
  its number everywhere, a short decimal prints as itself and any other ratio as the fraction, and a mix item that is no six-place
  decimal is an error. A time past its section's end, an lfo with no depth on a cutoff and an `e()` of a shadowed part are a warning
  or not counted. The ontology's rhythm names are said only to a program that has a `#! genre:` or `#! tags:` line; a program has at
  most 64 tags. `then` over parts is a sequence: the law is on the text and the music follows the flat text. *Each is the reading
  that leaves the pinned fixtures and every old program's CID alone; the long notes of chance, arithmetic, timing, shape, linked, meter and
  algebra say why.*

**Sets (the merge of origin/feat/btw-language, 2026-11)**

- The set was written on the base language while the extensions were written on the same base, and the two met at one commit. Where
  they could not both hold, the extensions' reading stayed and the set adapted (the rest of what a set decided is in "Sets"): a
  time is `Num Unit` tokens (no `Secs` token; `h` joins the units), a word of a set is a name outside the set (no new reserved
  word), and a set's control lines use the extensions' sub-bar timings (they are moved by the track's base like a bar-line
  timing, and bounded by the same window). *Every saved program the old build checks still gives the same canonical text and CID
  (the 23 of `tools/diffold.py`), the showcase's CID is the one it had, and the CIDs `tests/sets.rs` pins did not move.*
- The change plan counts a tempo-only change as one change (`1 changed`), so the folded panel line and `plan_summary` never say
  "no changes" while `tempo curve changed` is listed.

**Form extensions (sets, sections, parts, numbers; docs/beatscript/forms.md "Extensions")**

- A set in a role is its tracks, not its mix: the form's own sections stand between the tracks, so the mixes and cuts stay behind,
  said in a note. A playlist (an account's list, changed in place) is no source; a set, pinned by CID, is.
- A section where a line goes is the role's own part of it, as it first plays (the stored program's first stretch, the same
  `stored.rs` reads), written out as notes; a seed role takes each of its parts from it. A form's section is computed in the form
  written out on its own, in its own key and tempo, so the program's tempo leaves its notes alone and its key moves them.
- After a program that is no set, `.track2` is its section `track2`, as it always was; only a set reads it as a track.
- `a.bass=` wins over what `a=` gives the bass; only two bindings of the very same part are an error. The canon's `delay` counts
  beats and wraps the leftover beats in the bar, so the copy stays made of the subject's bars; `interval` counts semitones, as
  `transpose` does. The quodlibet's tunes are written `tune_a` and `tune_b` (a let named `a` reads as a note).
- *No reference that says none of them moves*: its written-out text is pinned (`roles.rs` `bound_forms_are_pinned`, blessed before
  the change, and `templates_v1_are_pinned`), and the examples keep their CIDs (`cid_pins.rs`).

**A recipe's nested parts and its stored expansion (roadmap 6680)**

- A part of an expansion is a role its template reference binds (a form in a role has its own), and its CID is of what it plays:
  its binding's lines written out all the way under the key and tempo it was written into, canonical. The parts are beside the
  CID: every program keeps its canonical text and CID, and its written-out text is the one it was. *One CID a part, of the notes,
  as the expanded CID is of a whole program; roles.rs records each binding as it writes it, so no part is read back from text.*
- A program with references is stored with its expansion beside it, a program of its own with the recipe's licence (CC0 by
  default), and each names the other in its meta; lineage and the JSON-LD say `expandsTo`, `expandedFrom` and `isBasedOn`. *The
  result a recipe makes is then a CID anyone can load and play without the generator or the templates, and its provenance is
  the recipe's.* It is a copy, so it is stored only when everything it copies is CC0 or the recipe author's own: *the site never
  publishes another author's work under rights that author did not give.* The long note is docs/beatscript/ext-template.md, "Nested
  parts" and "The stored expansion".

## Not done

What the language does not do, as honestly as the long notes say it. (Each feature's note has the full list.)

**The syntax review (what was found and left)**

- The canonical text of a def's body keeps the spacing its tokens had: a comparison written `$i> 1` or `$x <3` and `lp(300 + $i*4)`
  print as they were typed. The printer works on tokens, and a `<` is a comparison or the bracket of `<a b>` only to the parser;
  spacing them needs the body parsed (SYN-25). The text is stable and the CID is of it, so equal programs spelled with other spacing
  have other CIDs inside a def. The fix is to print a def's body from its parsed statements, which would move the CID of every
  saved program that has a def.
- `swing 0.1 * 2` prints `swing 1 / 10 * 2`: arithmetic in a setting is printed, not folded (the canonical-folding work).
- `deg(1, 3, 5)` (a comma and a space at the top of a `deg( )`) is three octave-down degrees, not a chord (a warning says so when
  the whole `deg( )` is a comma list); the chord is `deg(1,3,5)` or `deg([1, 3, 5])`.
- Error recovery skips to the end of a statement and its block; a `}` missing in the middle of a program still reports where the
  block began and not where it should have ended.

**Sets**

- a transition is a built-in (`crossfade`, `filter_exit`, `echo_out`, `cut`): a `std/transitions` def is not
  possible yet (there is no standard library, and a def's body is control lines, not fade curves). The three fades are
  approximations by velocity (see Sets). Tokens inside a track are not lit while a set plays (their spans belong to the
  track's text, not the set's), and a track's `evolve` generations are not shown or reachable by `btw generation`. A set has no
  seek: nothing in BeatScript jumps to a bar of any program (the arranged timeline is the thing a seek would use). A set link carries
  its tracks (never a fetch), up to 512 KiB of them. A track that
  `loop`s repeats its `At` control lines only the first time round.
- A mix ramps the tempo in a straight line (no `exp` or `s-curve`), and the map holds a track's own curve only where the track
  plays alone: a curve that is under way where a mix begins is cut there (the mix governs) and one that began in the overlap is held at
  its value where the overlap ends. A looping track's tempo curve is not repeated round the loop. A sung line (`sing`) of a track
  keeps looping on the loop row after its track ends (it is stopped when the set stops), a track's `mix { }` that is set outside a
  section is not put back at the track's end, and `assert length` takes minutes and seconds, not hours. A set holds no `assert` of
  its own: the tracks' are checked when they compile.

**The base language**

- A pattern is fixed at sixteenths: no triplet grid (a term count that does not divide the cycle snaps, with a
  warning), no swing per part; a cycle (`/N`) is a whole number of steps.
- An evolved generation is not saved as a program of its own, and a rule cannot add or remove parts. Lets belong at the top.
- A removed `deck` line pauses the deck; there is no "fade it out". The end is a stop on the bar line: the fade or
  echo is the outro's own lines.
- `at bar` and `every` count program bars (a song's own, with sections), not the setlist's.
- The net is compiled on the window's thread; a compile that takes longer than the rest of the bar lands a bar late
  (docs/SETLIST.md, "The dropout"). It is not measured on a long real run yet.
- A control line that needs a state it cannot see fails on its bar and says so in the window's words.
- No MIDI-note or audio-file sources. A restricted sample is checked and its rights kept, but the engine has no
  sampler for a user file on a raw net: a part that names one plays its `else` voice, with a warning.
- A very long *chain* costs a stretch at a time, a very long *net* (above 64 bars) is not possible.
- A link's signature is carried, not checked: the wallet's to make and check.
- A per-part filter or resonance: parts share their deck's insert.
- On the web (beats-bitwrap-io `/p`): the page decodes a link and checks its CID; playing it needs BeatScript compiled to
  wasm, which `djlive` is ready for (pure, no I/O) but nothing has built yet.

**The offline render** of `djctl btw render` and `btw test` carries the nets, the tempo curve, `duck`, `mix` and a
performance's mixer calls, and nothing else of the control lines: no filter, insert, `do` call, ramp, lfo, `move`, `load`,
`queue` or `sing` (the arrangement format has no op for a registry call). `beatscript render` and `beatscript listen` play the moves
too, all but a `vocoder_handoff` and a seed step, and in two readings: the control net starts `filter_exit` and `echo_out`
on the bar line their line fires on, the booth's scheduler starts the other moves on the next line after it, as the
booth does. A render holds a tempo ramp inside one net at ±50 % of the net's own tempo, and a program that has no end
renders 16 bars unless told. A level assertion about a filter sweep therefore measures a
render without the sweep, and a performed level, filter or `do` line is not in it whatever the inputs read. A deck
that comes in at a section starts on the section's bar line (it was started a beat ahead, so its notes fell a beat early).
The mix snapshot has no mixer to read there and goes back to the engine's starting positions.

**Controllers.** `held(key)` is the lowest key held over 127, so the lowest MIDI note (0, below any keyboard of the supported
controllers) reads like no key; a note-off lost because the controller was unplugged leaves `held(c4)` true until the next
press and release. A deck's tempo restored on stop is the deck's *effective* BPM (the fader's included).

**Arithmetic and guards.** Arithmetic in an `evolve`'s own arguments (`transpose (1 + 1)`) and its `when gen`: they
stay literals, because numbers stand side by side there. Arithmetic on pitches, keys and patterns (`c2 + 5`). Guards
on `let`, `def`, `section`, `tempo`, `key`, `swing` and an `evolve` rule's lines. No functions beyond `min` and `max`
(`floor`, `abs`, `round`), no strings or string building. A bare `x?1/4` is not read (write `x?(1/4)` or give the def
`0.25`). `let r = 3*2` is the number 6 while `let pat = 1*4 3*2` is a pattern (two terms: a value of more than one
expression is a pattern); a `let` of arithmetic that is not wanted as a number says which `let` it was where it fails
(`bass pat`), but is not refused. The tokens of one statement are not capped: a call with ten thousand arguments is read
(linearly), and only what recurses is bounded.

**Time and tempo.** A program with no sections has its bar 0 on the bar it was evaluated in, so its timed lines fall one bar
later in the offline render and in `btw test` (which start the music on bar 0) than they fire live when a deck was already playing
(a song is the same in both; the tempo curve is counted from the first bar you hear in both; "Time words"). Sections, generations and `repeat ... every` are whole bars (no `section drop 1/2 bar`); a def's
call takes no time between bar lines; a `move`'s `over` is whole bars. The editor draws no progress bar on a tempo
line and `get` does not list the tempo map's pieces, only the tempo now. No `tempo` inside an `evolve` rule, and no
tempo that follows a deck (a tap, a sync): the program's tempo is the master. No lfo of a tempo, a pattern or a part
param (`vel=lfo ...`), no shape on a `move` or `swap pairs`, no random wave (it would need a seed in the text). A finished
tempo ramp's last tempo is sent again every bar until the program stops, so a manual nudge of a deck's tempo is put back
within a bar (to move the tempo by hand, stop the program or give it a plain `tempo N`, which is in its nets and not
driven; R-22). `stop` and `end` put the tempo back only for a deck whose tempo the host could say when the program was
applied (a deck with nothing on it is left where the ramp left it).

**Keys, spelled two ways (L-19).** `am` and `a:minor`, `a` and `a:major` play the same and are four programs, each a CID of its own
(and `sing key=` and `assert key ==` inherit it). Folding the aliases would move the CID of every existing program that uses
the `a:minor` spelling, so it is deferred: the fix is to print a key's mode in one canonical word (`am` for the natural minor, a
bare `a` for major: the short spellings every old program uses) and to reread the long ones as them; it needs the pinned fixtures
checked and a note in "Decisions" for the programs it moves.

**Hot reload.** An edit that changes only a line's context (a section's length under an `lfo` with no `over`, a held `for N bars`
that was computed from it) keeps the running line's state, because the line's key (its section and canonical text) did not change:
the lfo stops, and a put-back falls, where the *old* program put them (L-20, R-05: reproduced on the fake clock, an edit of
the section from 8 to 16 bars leaves the lfo ending at bar 8). The fix is in `runtime::apply`: when a kept item's effect, timing or
hold differs from the new control's, re-derive its `undo_at` and `LfoRun` stop (or drop the run and let the line fire again).
It is the runtime fixer's (R-05) and is not in this run.

**Degrees and meter.** A numeral in a drum part is an error, not a hit. Voice leading does not run across the loop
point or from one section into the next and does not look ahead. No `/9` inversion or figured bass (`I6` is the added
sixth), no secondary dominants (`V/V`: the slash is an inversion only), and numerals do not follow a chromatic
alteration of the scale (the melodic minor's raised sixth and seventh: write `#vi°`, `bVII`, or use `a:harmonic`). A
section's key is not in `btw get` yet. The runtime shows no marker for where a part's own cycle begins. The
traditional names of rhythms are not in the ontology, so none is shown.

**Chance.** No chance on a control line, a section or a whole `deck` line, only on pattern terms; no chance in a
`choose` weight; `?` after a rest is allowed and does nothing (and is not printed); `maybe(p, 0)` and `maybe(p, 1)` are calls and are
not folded to a rest or to `p` as `x?0` and `x?1` are. The window does not mark a hit a chance dropped (it is
not there to mark).

**Composition and ports.** `stack` and `then` are over sections and parts only: no composition of control lines, and
`stack(a, b) oct=2` is an error. The interchange law and the other equalities of music are not normalised. `import`
does not bring a composition `let` (or sections, or assertions). A section with an `evolve` cannot be stacked. A stack
of unequal sections reloads a deck's net at the boundary when the alternation's phase moves, and the playhead carries
one header (the one that ends last). `after NAME` of a section that plays more than once is still an error. Two ports
of one type on a side (they would need names), parameters in an `emit` or the producer's arguments in a wire, a
default for an unwired input, a wire inside a section or a def, and feedback (on purpose) are not in. Arithmetic in a
`bars` or `tempo` emit is not range-checked. No editor feature (a jump from a wire to its def); the wiring is not in
`btw get`.

**Mix and sing.** A duck keyed by a part that is not low (a snare, a hat, a melody) needs a detector or a hit-keyed
gain stage in the engine. An attack that is a parameter of the pump, and a release in milliseconds rather than a note.
A strip pan. `mix` inside `evolve` rules. **No neural voice**: the engine, its models and their pinned download do not
exist yet (`vocals::NEURAL_ENGINE_BUILT` is `false`); the fallback is what runs. A sung line is not stopped when its
section ends (it loops on the loop row until stopped), starts on the loop row's own boundary, does not follow an
`evolve` (it sings generation 0), and has no melisma, no word split over a rest, and only English with the
dictionary's own spelling-to-sound rules.

**Assertions.** The window draws no panel of assertion states and `check`'s answer carries the pending notes as
`diagnostics`, not an `assertions` list. A level is measured per deck strip and the master, not per part or instrument
(`A.kick`). No assertion on `tempo` or on the number of parts, and no `assert` inside a section for its own bars (use `in`).
The constraints on notes are `never beside`, `never during` and `ends on` only (no distance in steps, no chord shapes); a
vocal timed `every N bars` or by a key cannot be judged and says so.

**Launch.** No latched pad: a tap shorter than a frame of the booth is missed. No per-device bank (`launch drop on pad(2)` is the first attached device's pads, as `pad(2)` is), no `launch` inside a block or a section, and no scene that carries a generation or a mutation over. The page and the phone have no pads: the song plays as written.

**Performed inputs.** No input in an `evolve` rule's number (deferred, with the reason in "Decisions"), in a `let`, in
a pattern, or in a guard on a `def`'s call. A pad is not latched: a tap between two bar lines is missed. No knob and
pad banks for the Inpulse and the TD-3, and no per-device choice of bank (`knob(3)` of the second device). A take that
begins mid-program replays from bar 0, and a performance of a program that was edited while it played is a take of the
program running at the end of it. The window does not show the performance (the Map shows the controls glowing): the
verbs and `btw get` do.

**Links to the ontology.** No refresh from inside the app (the script refreshes the snapshot; a refresh that drops a
term leaves old programs readable but their next save asks for another). No folk rhythm names. A tempo that moves is
not in the JSON-LD (`tempo` is the program's own setting). No SHACL or OWL validation, and the `djrust:` namespace is not
published (the `@context` is: `https://beatscript.app/context/v1.jsonld`; `btw jsonld --inline` spells it out). No genre-aware checks (tempo against the genre's `bw:bpm`, key against
its scale). The window does not show the links.
