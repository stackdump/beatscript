# F5: scale degrees, Roman numerals, a key per section, voice leading

*Extension F5 of docs/BTW-EXTENSIONS.md. Code: `crates/djlive/src/scale.rs` with small hooks in `lex.rs`, `parse.rs`, `ast.rs`,
`print.rs`, `pattern.rs`, `expand.rs`, `compile.rs`, `highlight.rs`. Tests: `crates/djlive/tests/ext_scale.rs` (numerals in calls:
`roman_calls.rs`). Examples: `examples/btw/scale.btw`, `examples/btw/progression.btw`.*

```text
key am
let tune = deg(1 b3 5' ~ #4 b3 1, ~)          # degrees with accidentals and octave marks
deck A: harmony i VI III VII voice-lead       # numerals, voiced for the least motion
deck B: bass i VI III VII | melody tune
section lift 8 bars {
  key cm                                      # the section is in C minor: every degree and numeral is read again
  deck B: arp i bII V7/7 i
}
```

## Grammar (EBNF)

```ebnf
(* a step of a pattern *)
step        = ... | degrees | roman ;
degrees     = "deg" "(" dstack ")" ;                     (* one slot, its terms squeezed into it like [..]; stacks as in a group *)
dstack      = dseq { "," dseq } ;                        (* the stack comma: glued to what follows it *)
dseq        = dterm { dterm } ;
dterm       = dleaf { "!" | "^" } [ "*" int ]            (* accent, slide, repeat, as any term *)
            | "~" | "x"
            | "[" dstack "]"                             (* a group *)
            | "<" dseq ">"                               (* an alternation: one a bar *)
            | "(" dterm ")" ;
dleaf       = [ "b" | "#" ] int { "'" } { "," } ;        (* 5' up an octave, 5'' two, 1, down one; b3 flat, #4 sharp *)

roman       = [ "b" | "#" ] numeral [ quality ] [ "/" ( "3" | "5" | "7" ) ] ;
numeral     = "I" | "II" | "III" | "IV" | "V" | "VI" | "VII"        (* upper case: a major chord *)
            | "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" ;      (* lower case: minor *)
quality     = "7" | "maj7" | "°" | "dim" | "°7" | "dim7" | "ø" | "ø7" | "aug" | "sus2" | "sus4" | "6" | "9" ;

(* an argument of chord( ) and arp( ): a chord is a symbol or a numeral, the numeral read in the key in force *)
arg         = ... | roman ;

(* settings and parameters *)
setting     = ... | "key" key ;                          (* also as the first statements of a section's body *)
param       = ... | "voice-lead" [ "=" ( "on" | "off" ) ] ;   (* on a harmony (or arp) part; bare means on *)
```

Lexing (shared `lex.rs`, new):

* Inside parentheses `#` followed by digits is a word, a sharp degree (`#4`). Elsewhere `#` still starts a comment, and a comment after
  a `)` is a comment: `deg(1 #4) # #5 is not a degree here`.
* After a step on the same line, `#` glued to a whole numeral (`#iv`, `#ii°`, `#ivmaj7`) followed by a space, `/`, a closing mark, `,`
  `*` `!` `^` `|` or the end is a sharp numeral. `# iv` (with a space), `#intro` and `#iv.` are comments.
* Inside `deg( )`, a `,` glued to a degree (a number, a flat or sharp degree, or its `'` marks) and followed by a space, the end of the
  line, `)`, `]`, `>`, `*`, `!` or `^` is the token `OctDown`, the octave-down mark (`1,`, `1,,` two octaves). Any other comma is the
  stack comma (`[1,3,5]`, `[~, 1]`). Outside `deg( )` every comma is the comma it was.
* `ø` (U+00F8), like `°`, belongs to the word it follows (`viiø`).

## Degrees: `deg( )`

`deg(...)` is one step: its terms share one slot as the terms of `[...]` do (`melody deg(1 3) 5` is two slots, the group's notes in the
first). Inside it every step is a degree of **the key in force** (the program's, or the section's), counted in that key's own scale
(`key am` is the natural minor scale, `d:dorian` dorian ...):

| written | is |
|---|---|
| `3` | degree 3 of the scale (8 is the tonic an octave up, to 28 as plain degrees always were) |
| `b3` / `#4` | that degree a semitone lower / higher. **Of the scale's own degree**: in `am` `b3` is B (C lowered), in C major it is E flat |
| `5'` `5''` | an octave up, two octaves up |
| `1,` `1,,` | an octave down, two |
| `~` `x` | a rest; a hit (the part's root, as in any tonal part) |
| `[1,3,5]` `<1 3>` `(5)` | a stack (a chord), an alternation (a degree a bar), a term in parentheses |
| `1!` `3^` `5*2` | accent, slide, repeat |

A bare `b3` outside `deg( )` is the note B in octave 3, as it always was. `#4` outside parentheses is a comment. The marks net out
(`5',` is `5`). `deg(...)` can be a `let`, an argument of a transform (`transpose(deg(1 3 5), 12)`) or in a `def` body.

A degree 0 is an error ("a degree counts from 1"); one above 28 is an error ("outside the scale", at its span).

## Roman numerals

A numeral is a step of `bass`, `melody`, `harmony` and `arp` parts (in a drum part it is an error). It is a chord built on a degree of the
**key in force**: `i` is the chord on degree 1, `VI` the chord on degree 6, in the key's scale (so `VI` in A minor is F, in A major it is
F#; `bVII` lowers that root a semitone, `#iv` raises the fourth's). **The case is the chord, not the scale**: `V` in A minor is the major
chord on E, `v` the minor one.

| quality | chord (intervals above the root) | factors |
|---|---|---|
| (none) | `I` major 0 4 7, `i` minor 0 3 7 | 1 3 5 |
| `7` | `I7` dominant 0 4 7 10, `i7` minor seventh 0 3 7 10 | 1 3 5 7 |
| `maj7` | `Imaj7` 0 4 7 11, `imaj7` (minor-major) 0 3 7 11 | 1 3 5 7 |
| `°` / `dim` | diminished 0 3 6 | 1 3 5 |
| `°7` / `dim7` | fully diminished 0 3 6 9 | 1 3 5 7 |
| `ø` / `ø7` | half-diminished 0 3 6 10 | 1 3 5 7 |
| `aug` | augmented 0 4 8 | 1 3 5 |
| `sus2` / `sus4` | 0 2 7 / 0 5 7 | 1 2 5 / 1 4 5 |
| `6` | `I6` 0 4 7 9, `i6` 0 3 7 9 | 1 3 5 6 |
| `9` | `I9` 0 4 7 10 14, `i9` 0 3 7 10 14 | 1 3 5 7 9 |

**Inversions.** `/3`, `/5`, `/7` name the factor that goes in the bass (`I/3` first inversion, `I/5` second, `V7/7` third): the
chord's lower tones are raised an octave until that factor is lowest. A factor the chord has not is an error (`I/7`, `Isus4/3`, `I6/7`)
that lists the tones it has.

**Roles.** What a part plays for a numeral:

* `harmony`: the chord's notes together, at the numeral's place in the bar;
* `bass`, `melody`: **one note, the chord's bass tone**: the root, or with an inversion the factor the slash names (`bass i VI/3` plays the
  third of VI under VI's slot);
* `arp`: the chord one note at a time, **four steps over the numeral's share of the bar** (`arp i` plays the root, third, fifth, root at
  steps 0 4 8 12; `arp i VI V7 i` plays four steps in each quarter bar). A 7th, 6th or 4-tone chord plays each tone once; a 9th chord
  leaves out its fifth.
* A pattern's octave (`oct=`, the part's default: bass 2, harmony 3, melody and arp 4) puts the tonic; the degree's root is then above it
  (`key.degree` semantics, as for plain degrees).

Keys of fewer than seven notes count numerals in the scale they come from (`c:pent` in C major, `a:minpent` and `a:blues` in A natural
minor); `deg( )` counts in the key's own scale as plain degrees do.

Numerals `I`..`VII` in either case (and every spelling the grammar gives them: `V7`, `bVII`, `ii°`) name a `let`, a `def` or a section too, and a `let` of the name wins in a pattern (BTW.md "Decisions"; they were reserved before the syntax review: no `let vi`,
`def V`, `section iv`).

## Numerals in calls

Where `chord()` and `arp()` take a chord symbol they take a numeral: `chord(V7, 3)`, `arp(VI, up, 8)`, `arp(bVII, updown)`, `chord(V7/7)`.
The rule is one line: **a numeral in a call is read as the same numeral is read as a step, in the key in force** (the section's own `key`,
else the program's, else C).

* `chord(V7, 3)` plays exactly the notes `harmony V7 oct=3` plays: the key's tonic in the octave, the root above it (as for steps, not
  by pitch class as `chord(f, 3)` does), a written inversion (`chord(V7/7, 3)`) with its tone lowest. `arp(VI, up, 8)` is the same notes one at
  a time over eight steps, in the part's octave (arp 4).
* A section's `key` moves it. `let arps = <arp(i, up, 8) arp(VI, up, 8)>` read in a section that says `key cm` is C minor's `i` and `VI`;
  `arp(am, up, 8)` is A minor wherever it is read. A numeral keeps its written colour in the new key (`i` stays minor from `am` to `c`).
* `voice-lead`: `chord(V7)` with no octave in a part that voice-leads is led like `chord(am7)`; with an octave it is voiced as written and is the
  chord before for the next. `arp()` does not lead (as with a symbol).
* A word that is both stays the chord symbol it was: no numeral spelling is a symbol, so no program changes meaning (a test says so).
* Through a def and a port: `band(VI)` into `arp($c, up, 8)`, `emit V7` into `arp($in, up, 8)` (BTW.md "Typed ports"); `emit <chord(i) chord(VI)>`
  into a `harmony $in` part. A progression a port takes as bare numerals is read by the part, as before.
* A numeral that does not read is said in numeral terms, at its line and column (`arp(VIII, up, 8)`: "`VIII` is not a numeral: a scale has seven
  degrees"; `chord(Imaj)`: did you mean `Imaj7`; `chord(I/7)`: "`I` has no seventh to put in the bass"). Any other word: "`foo` is not a
  chord", expected "a chord like am7, c, cmaj7, g7 or f#m7b5" or "a Roman numeral like V7 or bVII (read in the key)".

The generators print numerals: `generate` writes `let chords = <i VI III VII>` and `arp(VI, up, 8)` (and the pad `voice-lead=on`), and `genre_template`
writes the chords, the bass (`i ~ ~ ~ i ...`), the arp (`i*4`) and the tune (`i/5 ... i/3 ... i`) as numerals, so one `key` line moves a
whole generated song. (`seeded@2`: a seed makes a different program than `seeded@1` did; a stored program keeps its text and CID.)

## A key per section

```text
key am
section verse 8 bars { deck B: ... }
section lift 8 bars {
  key cm              # the first statement(s) of the body
  deck B: ...
}
```

A `key` that is the first statement (or the first statements, the last one wins, with a warning) of a section's body re-keys that
section: its degrees (plain and in `deg( )`), its numerals, `x` hits in tonal parts and the default key of `markov(density)`. It
applies to **the lines the song has outside every section too**: a part written at the top is read in each section's key. The program's `key`
is the default for sections that say nothing and still tags the share. `tempo` and `swing` stay top-level only; a `key` anywhere
else in a section, or in a `def` or `repeat`, is an error that says where it may stand. An unknown key is an error at its span, once, however
many times the section plays.

API: `Compiled::section_key(name) -> Option<Key>` (the key in force in that section, the program's where it sets none, C major where
the program has none; None for a name not on the timeline) and `Compiled::key_in(&Segment) -> Key`. `Compiled::section_keys` is the map.
**With `stack` (F10, `docs/btw/ext-algebra.md`, "Joins").** A stretch of a stack sounds in ONE key: the key of the first section playing in it that
sets one (in the order of the normal form), else the program's. Two sections that play at the same time and set different keys are an error
that names both (`sections `bridge` and `verse` play at the same time, in different keys: `key fm` (line 12) and `key am``), at the second `key`;
the same key spelled two ways (`am`, `a:minor`) agrees. A section with no key of its own follows the stretch it is in. The key of a section
is the key of the first stretch it plays in (its own, everywhere, where it sets one), so `section_key` answers every name `sections_at` gives, and also
a segment's name (`intro+verse`); `key_in(&Segment)` is the segment's own.

A net of a re-keyed section that plays pitches carries the key in its envelope tag (`DeckIr::key`, the roll's `djKey`, which the layer
moves read); a deck of drums keeps the program's, so its net is the same in every key and is not reloaded at the section line.

## voice-lead

`voice-lead` (or `voice-lead=on`) on a `harmony` (or `arp`) part: each chord after the first is voiced for the **least total semitone
motion from the chord before**; `voice-lead=off` is the chords as written. In the order the part is read (bar by bar, left to right):

* the first chord is the root position in the part's octave (a written inversion is kept: `I/3` first is E3 G3 C4);
* a later chord is tried in **every inversion** (only the written one if the numeral has a slash) and in **every octave from three
  below to three above** its written place (the MIDI range 0..127); the motion of a candidate is the sum of the distances the voices
  move, the notes taken in order (when the chords have different numbers of notes, the shorter chord's voices are matched in order to
  the longer chord's best notes, and a voice that comes in or drops out does not move: a dynamic program over the few notes);
* the least motion wins; **ties go to the lower inversion, then the lower octave**: the first candidate in (inversion, octave) order that has
  the least motion;
* `chord(am7)` or `chord(V7)` in such a part (no octave written) is led too; `chord(am7, 3)` with an octave is voiced as written and is the chord
  before for the next; raw notes (`[c3,e3,g3]`) are not chords and are left alone;
* each part and each section starts afresh (voice-leading does not run from one section's chords into the next section's), and the loop point of
  a one-bar part is not led (the first chord is the root position again).

Worked by hand (A minor, harmony octave 3, so A3 = 57): `harmony i VI III VII voice-lead` is A3 C4 E4, A3 C4 F4 (A and C stay, E rises
a semitone), G3 C4 E4 (A falls to G, F to E, C stays over them), G3 B3 D4 (motion 3). In C: `I V vi IV` is C3 E3 G3, B2 D3 G3, C3 E3 A3,
C3 F3 A3. These are in `tests/ext_scale.rs`, and the voicer is checked against an independent exhaustive search on 400 random cases.

## Canonical form

* `deg( )` prints its terms one space apart, accidentals and marks glued: `deg(1 b3 5' 1,)`. Marks that cancel are dropped (`5',`
  prints `5`); `(3)` prints `3`. A stack prints its comma glued to what follows (`[1,3,5]`), with a space before it where the term before ends in a
  comma mark (`[1, ,3]`), so the two commas read back as they were. This holds inside a `let` or `def` body too.
* A numeral prints with the canonical case of its quality: the letters of a quality lower case (`Imaj7`, `Isus4`), `dim` as `°`,
  `ø7` as `ø`, and the case of the numeral fixed where it says nothing (a diminished or half-diminished chord is lower case, `aug`
  and `sus` upper): `VII°` is `vii°`, `iaug` is `Iaug`. `I/3`, `V7/7`, `bVII`, `#iv` print as they are. Words in a `let` or `def` body and in a
  call's arguments are spelt the same way.
* `voice-lead` prints `voice-lead=on` (and `voice-lead=off` as written), among the parameters, sorted by name.
* A section's `key` prints as its first line: `key fm`.
* Existing programs keep their canonical text and CIDs (the fixtures are pinned, in `ext_scale.rs` as well as the old tests).

## Errors

All have a line, a column and, where a name was wrong, a did-you-mean:

* `Imaj`: "`maj` is not a chord quality after the numeral `I`", help "did you mean `Imaj7`?" (a quality within one letter of a spelling);
* `Iv`: "mixes upper and lower case", help "`IV` or `iv`"; `viii`: "a scale has seven degrees";
* `I/7`: "`I` has no seventh to put in the bass", help lists the inversions it has and `I7/7`; `I/4`: not an inversion;
* `kick I`: "a drum part has no pitch to give it";
* `deg(0 29 c3)`: counts from 1 / outside the scale / "`c3` is not a degree";
* `voice-lead=maybe`, `bass ... voice-lead`: what it takes and where it goes;
* a `key` in a section after other lines, in a def or a repeat; `key zz`: "is not a key".

A word that only begins like a numeral (`vel`, `inst`, `ivy`) is not claimed: the error it always had stands, and a `let ivy = ...` is fine.

## Decisions

1. **Degrees of the scale, accidentals relative to the scale's own degree** (the contract's reading): `b3` in A minor is B. For a
   minor key that wants the raised leading tone, `#7`.
2. **Numerals are degrees of the key's scale** (the chord on degree d), with the case giving the chord's colour, not derived from the scale.
   So `VI` in a minor key is the minor scale's sixth degree (F in A minor, major chord), and `bVII` is a semitone under the scale's own seventh
   (F# in A minor): in a minor key the usual subtonic is `VII`. In a five- or six-note key the numerals count in the seven-note scale it comes from.
3. **A single-note role plays the bass tone**, which is the root unless the numeral has an inversion; the contract said "roots in
   single-note roles". With no slash the two are the same; with a slash, playing the root would make the slash do nothing.
4. **`arp` plays four steps over its slot** and cycles the chord tones (as `arp(chord, up, n)` does); four steps land on the sixteenth
   grid for 1, 2 or 4 chords in a bar, so the snap warning stays quiet there. A 9th chord drops its fifth to fit.
5. **The case of qualities that fix it is normalised** (`VII°` is `vii°`, `iaug` is `Iaug`), so spellings of one chord are one program; the contract's "as
   written" is kept for the case that means something (major or minor, 7, maj7, 6, 9).
6. **`°7` / `dim7` and `ø7`** are accepted as well as the contract's `°` `dim` `ø`: the fully diminished seventh has no other name, and `ø7` is the common spelling.
7. **Voice-leading order is the order the part is read**, bar by bar, left to right: the same as playing order except under `reverse` /
   `rotate`, where it is the order of reading. A one-bar part's loop point is not led.
8. **`voice-lead` is accepted on `arp` as well as `harmony`** (both play chords; the led voicing is then arpeggiated). On other roles `voice-lead` is an error,
   `voice-lead=off` is accepted anywhere.
9. **Sharp numerals in the lexer** need something before them on the line and a boundary after, and a hash with a space after it is always a
   comment. A trailing comment written `#I like this` (no space after the `#`) is now a sharp numeral and a parse error: write `# I like this`.
   *A comment starts with `# ` (hash and a space); `#iv` is a sharp numeral.* The lexer cannot tell the two apart by what follows (a
   `let` name, a note or a chord symbol after `#iv` is just as plain a word as the English of a comment), so it does not try; the
   error says it instead. A word that is wrong on a line where a sharp numeral came before it says "if `#I` was meant as a comment:
   a comment starts with `# ` (hash and a space); `#iv` is a sharp numeral". A comment of words that happen to be notes (`#I a e`)
   plays them.
10. **The octave-down comma** is the token `OctDown` and its boundary also admits `>`, `*`, `!`, `^` (the contract lists a space, `)`, `]`): with
    only those a down-octave degree could not carry a repeat, accent or slide, and nothing else can follow a stack comma there.
    By the same rule `[1, 3, 5]` is three degrees in a row, the first two an octave down, and not a chord: the chord is `[1,3,5]`.
11. **A section's `key` retags the net** (`djKey`) only for a deck that plays pitches.
12. **`Compiled::section_keys` map, not a `Segment` field** (fewer shared lines to merge): `Compiled::key_in(&Segment)` reads it.
13. Numerals reserved are every valid spelling (`V7`, `bVII`, `ii°`), not just the bare `I`..`VII`. (Superseded: they name a `let`, a `def` or a section, BTW.md "Decisions".)

18. **A numeral is a chord of `chord()` and `arp()`** (`tests/roman_calls.rs`). The call took only symbols, so a `key` line moved the numerals of a
    progression and left the arps where they were, and `generate` and `genre_template` printed pitches. One rule rather than a new word (no `prog`
    statement or keyword: `let prog = <I V vi IV>` is that, and `prog` stays a plain name; no `roman(...)` call): where a chord symbol goes a numeral
    goes, read in the key in force. The inversion goes with it (`V7/7`, already one word in an argument). The symbol is tried first; no
    numeral spelling is a symbol. The numeral counts from the tonic in the part's octave, so `arp(VI, up, 8)` in A minor is above `arp(am, up, 8)`'s
    A: the generators print `arp` parts at the part's own octave, which measured within an octave of the pitches they printed before.

### Where F5 meets the other features (the integrator's decisions; tests in `tests/ext_cross3.rs`)

14. **Arithmetic in `deg( )`** (F2): `(` followed by arithmetic is a computed degree, as in any pattern: `deg(1 ($i + 1) (2 * 2 + 1)' 5)`. A
    parenthesised single degree (`(b3)`, `(5')`) is still that degree; an octave mark may follow the parenthesis (`($i + 1)'`). The arithmetic
    must be whole, 1 or more (`(7 / 2)` and `(2 - 2)` are errors that say so); an arithmetic that does not parse reports the arithmetic's
    own error. Without parentheses `$i + 1` is not arithmetic in `deg( )`: `$i` is put in place as a number.
15. **The comma mark is decided by the innermost parenthesis**: `1,` is an octave down only directly inside `deg(`, so a call in it
    (`deg((min($i + 1, 3)) 5)`) has the comma it always had. `lex::touches` lists `OctDown` as well as `Apostrophe`, so a macro or an
    import that puts degrees in place keeps `5'` and `1,` glued to their degree.
16. **A degree a macro puts in place is a degree**: `deg($p, 3)` (a space after the comma) is the degree `$p` an octave down, as `deg(2, 3)`
    is; the stack comma has no space, `deg($p,3)`. An octave-down degree is passed to a def inside `deg( )` (`tune(deg(1, 3))`);
    `tune(1,)` is not an argument. `#4`, `b3`, `5'` and `#iv` are passed as they are written.
17. **A `chords` port reads numerals as `harmony` does**: `ports::is_roman` is `scale::is_roman` (so `#iv`, `Vsus4`, `viiø7` are chords and
    `Vfoo`, `VIII`, `iVi` are not), and an inversion after a numeral is `/3`, `/5` or `/7` there too.

## Not done

* A numeral in a drum part is an error, not a hit.
* Voice-leading across the loop point and between sections, and a voice-leading that looks ahead (it is greedy chord by chord).
* Inversions `/9` and the figured-bass names (`6`, `64`); `I6` is the added sixth.
* Secondary dominants (`V/V`): the slash is an inversion only. Write the chord's own numeral (`II7`).
* Numerals do not follow a chromatic alteration of the scale (a raised sixth and seventh of the melodic minor): write the chord's own
  accidental (`#vi°`, `bVII`) or use `a:harmonic`.
* The key per section is not shown in `btw get` yet (read it with `Compiled::section_key`).
