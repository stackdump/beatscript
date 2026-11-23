# F11: typed ports on defs, `wire`, imports glued along ports

*Extension of BeatScript (docs/BEATSCRIPT-EXTENSIONS.md, section 3, F11). Code: `crates/djlive/src/ports.rs`, with small hooks in
`ast.rs`, `parse.rs`, `expand.rs`, `print.rs`, `highlight.rs`. Tests: `crates/djlive/tests/ext_ports.rs` and the unit tests
in `ports.rs`. Examples: `examples/beatscript/ports.beatscript` (self-contained) and `examples/beatscript/ports-import.beatscript` (an import; tested
with a fake snippet store).*

A `def` may declare a **boundary**: the values it needs (`in:`) and the values it makes (`out:`), each a *type*. `wire
changes.out -> walker.chords` glues an output of one def to an input of another, and the glue is **checked**: the types
must fit, every input is fed once, nothing may feed itself. A def with no ports is the macro it always was.

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

`walker(4)` expands to `deck B: harmony <chord(am7) ...> | melody markov(am, 0.5) oct=4`; `thump()` to `deck A: kick x ~ x ~ |
bass [a2 ~ a2 ~ f2 ~ g2 ~]`: the value `walker` emits flows into `thump` whether or not `walker` itself is called.

## Grammar (EBNF)

Notation as in docs/BEATSCRIPT.md. Additions to `def`, one new statement, one new line of a def body:

```ebnf
def         = "def" NAME "(" [ NAME { "," NAME } ] ")" [ "in" ":" type { "," type } ] [ "out" ":" type { "," type } ]
              "{" { line } "}" ;
type        = "chords" | "key" | "bass" | "melody" | "drums" | "rhythm" | "bars" | "tempo" | "pattern" ;
emit        = "emit" [ port ] tokens ;                       (* a line of a def body, at its top level, in a def with out ports *)
wire        = "wire" NAME "." port "->" NAME "." port ;      (* at the top of the program; the dot touches both words *)
port        = type | "in" | "out" ;                          (* a port is named by its type; "in" / "out" when it is the only one on its side *)
```

* In the body of a def with in ports, `$chords` (the port's type) is the wired value; `$in` is the same when it is the only
  in port. A parameter and a port may not share a name.
* `in:` comes before `out:`; each is said once; a type appears at most once on a side (`in: bass, bass` is an error: a
  port is named by its type).
* `emit` is the first word of a statement at the top level of the body. `emit bass ...`, `emit out ...` and `emit ...` name
  the port (the last only when the def has exactly one out port).
* `emit` is reserved; `chords drums rhythm pattern` are read as port types in a def's header and after `emit` only, and name a `let`, a `def` or a
  section elsewhere (BEATSCRIPT.md "Decisions"). (Originally: new reserved words `emit chords drums rhythm pattern`; the other port types
  and `wire in out` were reserved already).

## Types

| type | carries | what an `emit` of it may be |
|---|---|---|
| `chords` | a chord progression | chord symbols (`am7`, `fmaj7`), Roman numerals (`V7`, `ii°`, `bVII`, `I/3`), `chord(...)` calls, `~`, groups, alternations, `*n` |
| `key` | a key | one key word: `am`, `f#m`, `d:dorian` |
| `bass`, `melody` | a pattern for that part | any pattern (notes, degrees, `x`, `~`, groups, alternations, generators, transforms) |
| `drums` | a pattern for a drum part | hits and rests: `x ~ [x x] e(5,8)`, and the calls that keep it so (`e`, `reverse`, `rotate`, `every`, `degrade`, `slow`, `fast`, `humanize`); a pitch is an error |
| `rhythm` | hits and rests that fit any part | the same as `drums` |
| `bars` | a number of bars | a whole number 1 to 256 (or arithmetic over numbers, which F2 reads and this does not check) |
| `tempo` | a tempo | a number 40 to 300 (or arithmetic) |
| `pattern` | any pattern | any pattern |

**What fits what.** An input of type `want` takes an output of type `have` when `want == have`, or `want` is `pattern` and `have` is
any pattern kind (`bass melody drums rhythm pattern`), or `have` is `rhythm` and `want` is `bass`, `melody` or `drums` (a
rhythm is hits and rests, so it fits any part). Nothing else: chords are not a key, a bass is not drums, a `pattern` is not
known to be a bass. This is `PortType::accepts`, a preorder with `rhythm` below `bass melody drums` below `pattern`.

`$name` holes in an `emit` (the def's own inputs) fit any type where the emit is written; the value they stand for is checked
again when it is made. A plain `emit $in` is typed where it is written: `def f() in: bass out: chords { emit $in }` is a type
error in the def.

## Semantics

**A def is a module, a call plays it.** Wires are between defs (by name), not between calls. A *call* `walker(4)` expands the
def's body as a macro does, with its parameters and, for each in port, the value wired to it. The `emit` statements are not
lines of the program. The value of an out port is made **once, on demand**, from the def's own text and the values wired to
its inputs, in the order the DAG says (a def reached only through a wire is never called, and does not play: only its out
values are made). So `thump()` alone is enough to bring `walker`'s bass line to `thump`, without `walker`'s lines.

**An `emit` is fixed by the def's inputs, not by its parameters.** `emit transpose($in, $k)` with `$k` a parameter is an error
("what a def emits is fixed by its inputs"). That is what makes an out value a function of the wiring: the same wherever the
def is wired and however many times it is called. A value that varies is an input (`in: key`), wired to a def that makes it.

**Substitution.** `$chords` / `$in` are replaced by the tokens of the wired value, exactly as `$param` is. A pattern, a chords
value or a bass line of more than one term goes in as **one group** (`[ ... ]`), as a `let` pattern does: `transpose($in, 12)`,
`<$in c4>` and `bass $in c3` mean what they read as. A single term (`c2`, `[c2 e2]`, `<c2 e2>`, `reverse(...)`), a key and a
number go in as they are (so `markov($key, 0.5)` is `markov(d:dorian, 0.5)`, `at bar $bars:` is `at bar 4:`).

**Chord symbols play.** A chords value of more than one term has each bare chord symbol written as `chord(x)` when it is made:
`emit <am7 g7>` flows as `<chord(am7) chord(g7)>`, so `deck B: harmony $chords` plays it (a bare symbol is no step of a
part). A lone symbol (`emit am7`) stays a word: `arp($in, up, 8)` and `chord($in)` take it. Roman numerals are left for the part
to read (F5), and what is inside a call is not touched. A lone numeral (`emit V7`) is a word too, and `arp($in, up, 8)` and `chord($in)`
take it, read in the key in force where the call plays.

**The wiring.**
* Both ends must exist (`did you mean` among the defs with ports), the source must be an out port and the target an in port,
  and the types must fit: `type error: `groove.out` is a drums pattern, `walker.chords` wants chords`, at the line and column
  of `groove.out -> walker.chords`.
* **Each input is wired once** (the same output may feed many inputs: fan-out). A second wire to an input is an error naming both.
* **No loops**, through any number of defs: `the wires make a loop: alpha -> beta -> gamma -> alpha`, at the wire that closes it.
* **An unwired input is an error where it matters: at a call.** `` `walker.key` is not wired: `walker` needs a key and nothing feeds it``,
  naming the def and the port, with `wire home.out -> walker.key` as the help when some def makes a key. A def that is
  never called does not need its inputs (a library of modules is fine).
* A wire in the program's own text whose consumer is never called is a **warning**, said only when some other def with ports is called.
* A chain of wired defs may be at most 64 long (an error beyond that; the wiring is a DAG, so this is a guard only).

**Imports.** `import "harmony"` brings the snippet's defs **with their port declarations** and its `wire` statements: a saved
composition keeps its wiring. The canonical text holds the snippet's CID, so the composition is of exact modules; the same
program by name or by CID is one program. `wire harmony_changes.out -> walker.chords` joins an imported module to the
program's own. An input already wired by the import cannot be wired again (an error naming both lines); a program that
wants a different wiring imports a snippet that does not have that wire. Errors inside an imported def are at its import line
(its tokens' spans are all there, as for lets and defs).

**The editor's marks.** A call's lines carry the call in their `via`; a hit that came out of a wired value traces to the `$in` /
`$chords` of the consumer's def body and to the call, like any `$param`. The new words are in `highlight.rs`'s keyword class.

## Canonical form

* `def walker(oct) in: chords, key out: bass {`: ports sorted by type (the contract's order: `chords key bass melody drums
  rhythm bars tempo pattern`), so `in: key, chords` and `in: chords, key` are one program. A def with no ports prints exactly
  as before (no CID moves).
* `wire changes.out -> walker.chords`: one space round `->`, no space round the dots. A sole port is written `in` / `out` in
  canonical text, and the type spelling of it is rewritten to that when the def is known (`wire changes.chords -> walker.chords`
  is `wire changes.out -> walker.in` when each has a sole port): one program, one CID. A name that cannot be found is left as
  written and the lowering says what is wrong with it.
* `emit` lines are body tokens, printed as the body is (one statement a line, canonical pitch spelling). `emit c2`, `emit bass
  c2` and `emit out c2` are three spellings of one meaning and three texts, as `$a` and `$b` for the same parameter would be.
* The order of `wire` statements is kept as written (it does not change what flows: see the laws).

## The cospan reading

A def with ports is an **open system with an interface**; `wire` **composes** two along the interface they share.

*Interfaces are objects.* The nine port types form a preorder 𝒯 (`have ≤ want` is `accepts`). An interface is a finite set of
ports, each typed, i.e. an object of FinSet/𝒯 (a port is named by its type, so on one side an interface is a subset of 𝒯).

*Defs are cospans.* A def D with `in: X out: Y` is a cospan in FinSet/𝒯

    X --l--> N_D <--r-- Y        N_D = X ⊔ Y  (one cell for each port),  l, r the injections,

decorated with what the body says: its lines L_D (with a hole for each x ∈ X, and the parameters) and its **emit function**
f_D : V(X) → V(Y), where V(t) is the set of token strings that fit the type t and V(X) is the product of V(τ x): "put the inputs into
the `emit` statements".

*Wire is composition.* A wiring W from D to E is a set of pairs (y, x), y ∈ Y_D, x ∈ X_E, with τ(y) ≤ τ(x), that is a
function on X_E (each input at most once) and is arbitrary on Y_D (an output may feed many inputs, or none). The composite
D ;_W E is the cospan whose apex is the **pushout** N_D ⊔_W N_E (the two cells of a wired pair become one cell), with the
unwired inputs of E added to its inputs and all outputs kept, decorated with the lines L_D ⊎ L_E and the function
(x, x') ↦ ( f_D(x), f_E( f_D(x)|_W , x' ) ). Fan-out is the copy of a cartesian structure (an output is used as often as
wired) and an unused output is a discard; there is no merge (an input has one source).

*What the language adds to cospans: no feedback.* In the full cospan category a wire from D's output to D's own input is a
trace, and is fine. Here it is an error (`the wires make a loop`), because a value that feeds itself has no meaning for a
macro that expands in finitely many steps: the wirings are the **acyclic** ones, a (cartesian) symmetric monoidal category of
modules, not the compact closed one. That is the same totality rule as for a `def` that calls itself.

**The laws claimed, and the tests that hold them** (`crates/djlive/tests/ext_ports.rs`):

1. **Associativity.** Wiring three modules A, B, C in a chain does not depend on the bracketing: (A;B);C and A;(B;C), and any
   cut of a longer chain, expand to the same lines. A saved snippet is a bracket (its defs and its wires), and importing a
   bracket and wiring the rest is the other bracket. *Test:* `any_bracketing_of_the_wiring_expands_to_the_same_lines`: random
   chains, every cut, the left part or the right part in a snippet, and three snippets wired together.
2. **Wires are a set.** The order the `wire` statements are written in (before the defs, after the call) does not change
   the expansion. *Test:* `the_order_the_wires_are_written_in_does_not_change_what_flows`.
3. **Identity.** For a type t the module `id_t` (`in: t out: t { emit $in }`) is the identity cospan (its apex is one cell and
   both legs hit it; f is the identity): wiring through it changes nothing. *Test:* `the_identity_module_is_invisible`.
4. **Monoidal structure.** Independent modules (disjoint apexes) are in parallel, and the order of their calls is the braiding:
   it changes the order of lines, not the lines or the nets. *Test:* `independent_modules_may_be_called_in_either_order`.
5. **Denotation.** The value at an out port of a composite is the composite of the emit functions, checked against an oracle
   worked out on the side. *Test:* `a_chain_expands_to_what_the_hops_make`.
6. **Loops are exactly the cycles of the wiring.** *Tests:* `the_loop_finder_agrees_with_a_topological_sort` (300 random
   graphs against a peeling oracle, every reported loop a real one) and
   `a_random_wiring_of_modules_is_a_loop_error_exactly_when_it_has_a_loop` (random programs).
7. **Canonical form.** Reformatting keeps the CID and the expansion; the canonical text is a fixed point; an edit changes the
   CID. *Tests:* `reformatting_keeps_the_cid_and_an_edit_changes_it`, `reformatting_a_random_wiring_keeps_its_cid_and_its_expansion`,
   and `examples/beatscript/ports.beatscript` through `tests/examples_beatscript.rs`.
8. **Conservativity.** A def with no ports is the macro it was: same canonical text, same CID, same expansion
   (`a_def_with_no_ports_is_the_macro_it_was`, and every earlier test and fixture).

## Errors, with their help

| when | message (line and column of the construct) |
|---|---|
| a type does not fit | `type error: `groove.out` is a drums pattern, `walker.chords` wants chords` |
| unknown def in a wire | `` `chnages` is not a def``, `did you mean `changes`?` |
| unknown port | `` `walker` has no input port `chord` ``, expected: `chords`, `key`; `did you mean `chords`?` |
| output used as input (or back) | `` `walker.chords` is an input: a wire goes from an output to an input `` |
| `in` where there are several | `` `walker` has several in ports, so name one: walker.chords or walker.key `` |
| input wired twice | `` `walker.chords` is wired twice: from `changes.out` (line 17) and from `other.out` `` |
| input never wired, def called | `` `walker.key` is not wired: `walker` needs a key and nothing feeds it ``; help `wire home.out -> walker.key` |
| a loop | `the wires make a loop: alpha -> beta -> gamma -> alpha` |
| out never emitted | `` `foo` declares `out: chords` and never emits it `` |
| out emitted twice / no such out | `` `foo` emits its `chords` out twice (line 2) ``, `` `foo` has no out port `melody` `` |
| several outs, no name | `` `foo` has several out ports: say which one this `emit` makes `` |
| `emit` in a block / in a def with no outs | `` `emit` belongs directly in the def's body, not inside a block ``, `` `emit` is for a def with `out:` ports, and `foo` declares none `` |
| a parameter in an `emit` | `` an `emit` cannot use the parameter `$k`: what a def emits is fixed by its inputs `` |
| a value not of its type | `` `foo` is not a chord: a chord is a symbol like am7, fmaj7, g7, or a Roman numeral like V7 ``, `` `c3` is a pitch: a drums pattern is hits and rests (x ~ [x x] e(5,8)) `` ... with `in the `chords` out of `changes`: a `chords` port carries ...` |
| bad header | `` `chord` is not a port type `` (`did you mean `chords`?`), `` `in:` is in the wrong place in the def's header `` (`write `in:` before `out:``), `` `bass` is already a port of `in:` (line 1) `` |
| a wire not at the top | `` a `wire` belongs at the top of the program, outside blocks `` |

## Decisions

* **Wires join defs, not calls.** The contract's `wire changes.out -> walker.chords` names defs, and "each input wired once" is
  per def: so a def is a module, one wiring, any number of calls. A call plays the lines; a wire carries values.
* **A producer that is only wired is not played.** Its out values are made; its lines are not in the program. A program that
  wants both calls it.
* **An `emit` may not use the def's parameters** (see Semantics). The alternative (the producer's args from its call) makes a value
  depend on which call and in what order was expanded first, and breaks the laws above. An input port does the same job.
* **Pattern and chord values go in as one group.** The same rule a `let` pattern follows, so `$in` is a unit wherever it stands;
  a single term and keys and numbers go in as written.
* **Bare chord symbols in a progression become `chord(x)` calls** when the value is made, so that the progression plays in a
  `harmony` part with no ceremony; a lone symbol stays a word for `arp(...)` / `chord(...)`. The contract says a chords port "takes
  chord and Roman-numeral terms"; the type check reads any word that is a chord symbol or a Roman numeral as F5 reads one
  (`scale::is_roman`; first written to take anything that started like a numeral, made exact when F5 landed), and an inversion
  after a numeral is `/3`, `/5` or `/7`.
* **Port names.** Per side a port is named by its type, or `in` / `out` when it is the only one; both spellings are read, one
  is printed. With several ports on a side `in` / `out` name none of them (an error saying which to write).
* **Ports are sorted by type** in the canonical header: the order of the ports is not meaning.
* **`rhythm` fits `bass`, `melody` and `drums` inputs.** The contract says types must match and that `pattern` takes any
  pattern kind; a rhythm is hits and rests only, which every part reads, so it is accepted below them. Nothing else is.
* **`bars` is 1 to 256 and whole; `tempo` is 40 to 300** (the language's own ranges for a section and a tempo). A value that is arithmetic
  (F2) is accepted unchecked: it is read when it is used.
* **`drums` / `rhythm` are hits and rests**: a pitch in one is an error (a `let` pattern is looked through).
* **An unwired input is an error at a call, not at the def.** A library of modules (or a snippet) has many inputs nobody
  has wired yet. The wire statements are checked wherever they are.
* **A snippet's wires come with its defs.** Without them a saved composition would lose its wiring, and "any bracketing" could
  not be saved. An input wired in an import is wired.
* **A type per side once.** A port is named by its type, so two `chords` inputs would be unnameable; name them with two defs.
* **New reserved words**: `emit chords drums rhythm pattern` (a `section` or `let` of those names would have been a clash). (Superseded by the syntax
  review: only `emit` is reserved; the port types are read as such in a def's header and after `emit`, BEATSCRIPT.md "Decisions".)
* **Type errors say `wants`** for inputs and `is` for outputs, with the spelling the wire was written with (a sole port written
  `walker.chords` is reported as `walker.chords`, not `walker.in`).
* **The warning for a wire whose consumer is never called** is only said when some other def with ports is called: a library of
  modules and wires, called by nothing, is not a mistake.

## Not done

* Two ports of one type on a side (they would need names: `in: chords lead, chords bass`).
* Parameters in an `emit`, or the producer's arguments in a wire (`wire changes(4).out -> ...`). An input does the job.
* A default for an unwired input (the program's `key`, say): an unwired input is an error.
* A wire inside a section or a def body (a wire is a top-level declaration, like `import`).
* Feedback: `wire a.out -> a.in` is a loop and an error, on purpose.
* Arithmetic in a `bars` / `tempo` emit is not range-checked (F2 reads it when it is used); Roman numerals are only checked for
  their numeral (F5 reads the rest).
* No editor feature (jump from a wire to its def); the wiring is not in `btw get` yet.

## For later workers

* `djlive::ports`: `PortType` (`name`, `parse`, `accepts`, `is_pattern`), `Ports` (`ins`, `outs`, `find`, `name_of`, `names`),
  `Port`, `Dir`, `PortRef`, `WireStmt`, `wiring_cycles(edges)` (loops of a wiring, pure), `is_chord_word`, `is_roman`.
* Shared-file hooks: `DefDef.ports`; `StmtKind::Wire` (last variant); `Imported.wires`; `parse.rs`: `Parser::def_ports` in the
  `def` arm, `Parser::wire_statement` in the `wire` arm, `ports::canonicalize` after the import pinning, `resolve_import` carries
  wires and sets the ports' spans on the import line; `expand.rs`: `Lower.ports`, `ports_setup` after `call_cycles`, `port_call` in
  `invoke` (its body is `def.body` without its `emit` statements, and `subs` gets the in values), `port_warnings` at the end of
  `run`, `StmtKind::Wire` in `stmt` (an error: not at the top); `print.rs`: `Ports::header` in the def line and `WireStmt::text`.
* `ports::split_emits(body)` cuts a def body into its `emit` statements and the rest; `Lower::fits(ty, tokens)` checks a value against
  a type (with `$` holes allowed); both are reusable by a feature that wants typed values.
