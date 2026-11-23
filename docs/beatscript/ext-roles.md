# Roles: a form's parts bound to sources

Long note for docs/BEATSCRIPT.md "Phrase templates", the roles paragraph (roadmap 6673, 6674, 6678), and "Name a source: `use`"
(roadmap 6668's canned and stock sections; the last section below). Code: `crates/djlive/src/roles.rs` (the role tables, the
reader of a binding, the writing out), `crates/djlive/src/template.rs` (the reference and its canonical spelling),
`crates/djlive/templates/cross.beatscript` (the cross form), `crates/djlive/src/uses.rs` (`use`). Tests: `crates/djlive/tests/roles.rs`
(no generator: themes, notes, forms in a role), `crates/beatscript-api/tests/forms_roles.rs` (seeds in a role, stored programs, eject),
`crates/djlive/tests/uses.rs` and `crates/beatscript-api/tests/uses.rs` (`use`).

## What it is

A form template (docs/beatscript/forms.md) is a structure: sections, an arrangement, a mutation plan. Until now its material was
fixed, the public-domain themes it was built from. A **role** is a named part of that structure that a **source** can fill:

```text
arrange from template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass)
arrange from template(canon, @mgm_pease_porridge_hot, subject=ode_to_joy, ground=[d2 ~ a1 ~ b1 ~ f#1 ~])
arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876))
arrange from template(cross, @psalm42_deep_calleth, a=gen@1 dubstep seed=54876, b=gen@1 house seed=7)
```

The form supplies the structure; the sources fill its roles. A role nobody binds keeps the form's own material, so a reference
with no roles is the text it always was (`templates_v1_are_pinned`, and the CID of every existing template program, do not
move). Binding a role is new text, and a new CID.

This is the structure half of the language (docs/BEATSCRIPT.md "Two halves"). Everything a binding writes is BeatScript a person could
have typed: a role is a `let` (a let a bar) or a run of sections, which the form's sections and its mutation plan read by name.
The one engine call among the sources, a seed of the generator, stays an engine call: it is written into the text as a `let`
or a `section ... from` line that the gen stage resolves after the template stage, pinned and versioned (`gen@1`), exactly as if
the program had said it.

## Syntax

```ebnf
template    = "template" "(" NAME "," ( PHRASE | STRING ) { "," binding } ")" ;
binding     = NAME [ "." role ] "=" ( source | [ "-" ] INT ) ;
source      = gen | NAME | notes | form | stored | track ;
gen         = GEN NAME "seed=" ( INT | NAME ) [ "bpm=" NUMBER ] [ "key=" KEY ] [ "length=" ( "short" | "standard" | "long" ) ] [ "section=" NAME ] [ "part=" role ] ;
notes       = "[" pattern "]" | "<" "[" pattern "]" { "[" pattern "]" } ">" ;
form        = "template" "(" NAME { "," binding } ")" [ "." NAME [ "." role ] ] ;
track       = CID "." NAME [ "." NAME ] [ "." role ] ;
```

(The part after a role, `a.bass=`, the numbers, a form's section and a set's track are the form extensions: docs/beatscript/forms.md
"Extensions".)

- Roles come after the phrase, as `role=source`, comma apart, on the reference's line.
- **Canonical spelling**: the roles in the order the form names them (the table below), `=` glued, each source in its own
  canonical spelling (a `gen` call as gen.rs spells it, notes as the printer writes them: `bb1` is `a#1`), `, ` between. The
  canonical text of the reference is one word to the parser, so `template( passacaglia,@eccl3_season,upper = ode_to_joy,
  ground=[g1 ~ bb1 ~] )` and `template(passacaglia, @eccl3_season, ground=[g1 ~ a#1 ~], upper=ode_to_joy)` are one program.
- A template inside another (a source) takes **no phrase**: the phrase is the outer form's, sung in its own section.
- `template` stays a plain name everywhere else, as before.

## The sources

| source | written | what the role gets |
|---|---|---|
| `gen@1 dubstep seed=54876 section=drop part=bass` | `let ROLE = gen@1 ... section=drop part=bass`, which the gen stage writes out as a let a bar | the part's notes in that section, generated in the key and at the tempo the role plays in |
| `gen@1 dubstep seed=54876` (a whole seed) | the same, the form filling in what it leaves out | a line: the role's own part (a ground's is the bass, a subject's the melody, the episodes' the arp) from the first section that plays it, the drop or the chorus first; a seed role (cross): its melody, bass and chords; a role of sections: the seed's sections, in its order |
| `ode_to_joy` (a theme of the public-domain library) | its bars as `let ROLE_1 = ...`, moved | the theme in the key the role plays in: moved to its tonic, or to its relative key when the theme's mode is the other one (Ode to Joy in G minor is in B-flat major), the nearest way; its cycle (`/20` for Mars) where only parts play the role |
| `[d2 ~ a1 ~]`, `<[d2 ~ a1 ~] [g1 ~ a1 ~]>` | a let a bar | the notes as written (a degree is the key's) |
| `<cid>.drop.bass` | `let ROLE_1 = <cid>.drop.bass`, which the stored stage writes out | the part, read in the key it lands in (stored.rs) |
| `<cid>.drop` | `section NAME from <cid>.drop` | in a role that takes sections: that section |
| `<cid>` (a whole stored program) | `section ep_NAME from <cid>.NAME`, one for each section it plays | in a role that takes sections: its sections in the order its arrangement plays them |
| `template(passacaglia, ground=...)` | its lets and sections, every name it defines given the role's prefix (`ep_`) | in a role that takes sections: the form, its plan spread over the places the role plays in |
| `template(cross).both`, `template(cross).both.arp` | a let a bar of notes; where sections go, the section written out | where a line goes, the notes the section's part (the role's own, or the one named) plays the first time it plays, moved into the role's key; where sections go, that section |
| `<cid>` of a set, `<cid>.track2` | `section ep_t1_NAME from <track>.NAME` for each track's sections | in a role that takes sections: the set's tracks in order, each its sections, one track a place; `.track2` is that track's program |

A role is a let a bar: `NAME_1` ... `NAME_k` and `NAME = <NAME_1 ... NAME_k>`, so `~ NAME_* rot 2` mutates every bar, as with a
theme. The default's bars named elsewhere in the plan follow: `morph mountain_king_3 0.2` with a two-bar subject is
`morph dux_1 0.2`, the bar the line comes round to.

## The forms by role

| form | role | what it is | default | written as |
|---|---|---|---|---|
| passacaglia | `ground` | the bass line under every section, never changed | Holst's five-beat Mars ostinato, `mars_ostinato` | `ostinato` |
| | `upper` | the line above the ground, changed a little more every pass | the form's own line in G minor | `upper` |
| canon | `subject` | the line chased by copies of itself, a bar and a beat behind | Pachelbel's first violin, `pachelbel_violin` | `subject` (the copies: `follow`, the last bar first; half a bar behind for a one-bar subject) |
| | `ground` | the bass under the chase | Pachelbel's ground | `ground` |
| rondo | `refrain` | the tune that comes back, changed on each return | Joplin's Entertainer | `tune` |
| | `episodes` | what plays between the refrains: a line, or sections | Bach's Prelude in C, the episode's figure | a line: `figure`; sections: in place of `episode`, over its places |
| fugue | `subject` | the theme that enters, is answered a fifth up, re-enters transposed | Grieg's Mountain King | `dux` |
| | `counter` | the countersubject, drawn toward the subject | Bach's Badinerie | `countersubject` |
| theme-and-variations | `theme` | the one source every variation changes | Dvorak's New World Largo | `air` |
| quodlibet | `a`, `b`, `ground`, `third` | the first tune; the second, drawn toward it; the bass; the third line both are pulled toward | Pachelbel's violin; Ode to Joy; Pachelbel's ground; the fate motif a semitone down | `tune_a`, `tune_b`, `ground`, `third` |
| cross | `a`, `b` | the two sources: melody, bass and chords (a whole seed; a line is the melody) | Ode to Joy over Pachelbel's ground; the Badinerie over a B minor bass | `a_melody` `a_bass` `a_harmony`, `b_melody` `b_bass` `b_harmony` |

A let cannot have a section's name (`ground` is a section of the passacaglia, `refrain` of the rondo, `subject` of the fugue,
`theme` of the theme and variations), so those roles are written under another name: `ostinato`, `tune`, `dux`, `air`. The
quodlibet's `a` and `b` are written `tune_a` and `tune_b` (a let named `a` would read as a note); its `third` is the fate motif
moved a semitone by hand, so binding it also takes away the motif's own lets.

## The cross form

`cross` (roadmap 6673) is a quodlibet skeleton for two sources: `a_leads` (a's tune over its own bass and chords), `b_leads`,
`both` (b's tune over a's bass, a's tune under it as an arp) and `sung` (the phrase over the crossing), and an arrangement that
morphs a toward b and back in cycles: `~ a_melody_* morph b_melody_1 0.25`, `~ b_bass_* morph a_bass_1 0.25`, an
`@ a_melody_* xfer b_melody_1` one-shot in the overlap, then the pull turned round, `loop 3`. Two seeds of any genres cross
cleanly because each is generated in the program's key and at its tempo (gen.rs fits a call that says neither): their own keys
do not matter. Unbound, it is Ode to Joy over Pachelbel's ground crossed with Bach's Badinerie in B minor, D major's relative
minor. Example: `examples/beatscript/cross-two-seeds.beatscript`.

## Forms in forms

A role that takes sections (the rondo's `episodes`) can be bound to another form, and templates compose:

```text
arrange from template(rondo, @shanty_drunken_sailor, refrain=gen@1 dubstep seed=54876 part=melody,
    episodes=template(passacaglia, ground=gen@1 dubstep seed=54876 part=bass, upper=gen@1 dubstep seed=54876 part=arp))
```

(one line in a program). The inner form is written out with the outer phrase, then its vocal is taken out (its sung section,
the vocal's lets and one-shots), every name it defines is given the role's prefix (`ep_ground`, `ep_upper_*`, `ep_drm_*`), its
own roles are bound, and its sections open with its own `key` when that is not the program's (`key gm` in a rondo in C); its
seed calls say that key (`key=gm`), so they fit the sections they play in. Its arrangement becomes plays (a section and the
mutation lines after it), shared out in order over the places the outer role plays in: the rondo's two episodes get the
passacaglia's ground and first variation, then its other two variations, its cumulative changes carrying on across the loop.
Example: `examples/beatscript/rondo-of-passacaglias.beatscript`.

## Rules

- **Key and tempo.** A source plays in the key of the sections it fills. At the top that is the program's (its own `key`, else
  the form's): a `gen` call is fitted to it, a theme moved into it. In an inner form it is that form's own key. The tempo is the
  program's. (The form's own material keeps the form's key, as before: a program that says another `key` moves the vocal line
  and the bound sources, not the form's themes.)
- **Pinned and deterministic.** The same reference is the same written-out text and the same nets; a seed is pinned by its
  version and resolves to the same program (gen.rs); a stored program is named by its hash. The template stage resolves a seed
  once to choose its section and count its bars; the gen stage resolves it again, to the same program.
- **Doc comments.** Every section the form writes has its `#` line above it, a borrowed section too (`# ep_drop: the drop of
  gen@1 dubstep seed=54876`); each bound role says what fills it (`# ground, bound: gen@1 ...`), and the header names every role
  with its source or its default.
- **Total.** Forms nest **3** deep (`MAX_NESTING`): a fourth `template(` is an error at it. A line is at most **16** bars
  (`MAX_LINE_BARS`), and a role that takes sections takes at most **128** bars of them a pass (`MAX_SECTION_BARS`: a whole seed
  of standard length, not a long one). Each is an error at the source that crosses it. With every source bounded and the nesting
  bounded, a written-out form is bounded.
- **Errors at the binding.** An unknown role (with the form's roles and the nearest), a role bound twice, an unknown theme (the
  nearest id), a theme in 3/4 where the role's copies are made of 16-step bars, a whole form in a role that takes a line (one of
  its sections is a line), a phrase in an inner template, a groove (deep-house) as an inner form, a seed's section or part it does
  not have, a stored program no host has (`... is not here`), a form that takes no roles yet; and those of the extensions (a
  set's track it does not have, a part of a role that has none, a number out of its range: forms.md "Extensions").

## Eject, and provenance as a chain

`refs::sites` lists a template with roles as one reference (structure); its bound sources are inside it (refs.rs reads the later
stages with the template reference blanked). Eject writes the form out under `# from: <the reference> -> <CID>`, and the
reference *is* the chain: `# from: template(rondo, @shanty_drunken_sailor, ..., episodes=template(passacaglia,
ground=gen@1 dubstep seed=54876 part=bass, ...)) -> z4EBG9...`. The seed calls left in the written-out text are references in
their turn, each ejected under its own `# from: gen@1 ... -> <CID>` line. Every level plays the same (the nets are equal), and
`forms_roles.rs` holds it.

## Built since (docs/beatscript/forms.md "Extensions")

- **Sets in a role**: a stored set's tracks as a role of sections, each track's sections in turn (`episodes=<set cid>`), and one
  track by number (`<cid>.track2`). Its mixes stay behind.
- **A section as a line**: a section where a line goes gives the role its own part of it, a stored program's
  (`subject=<cid>.both`) or one section of another form (`subject=template(cross).both.arp`, the notes it plays the first time
  it plays); the canon's copies are made of its bars.
- **Sub-roles**: `a.bass=...` binds one part of a seed role, beside the whole role or alone.
- **The canon's `delay=` and `interval=`**: the chase's copy N beats behind, N semitones from the leader.
- **Quodlibet roles**: `a`, `b`, `ground`, `third`.

## Name a source: `use` (built)

`use SOURCE as NAME` is `use`'s second form (the first is `use restricted`): a standalone structural import, so a long nested
reference reads in lines, and a source used twice is named once.

```text
use gen@1 dubstep seed=54876 section=drop part=bass as walk
use template(passacaglia, ground=walk) as episode_form
arrange from template(rondo, @shanty_drunken_sailor, episodes=episode_form)
```

### Grammar

```ebnf
use         = "use" "restricted"
            | "use" use_source "as" NAME ;
use_source  = stored                 (* <cid>.drop  <cid>.drop.bass  <cid> *)
            | gen                    (* gen@2 techno seed=42 section=drop [part=bass] *)
            | "template" "(" NAME { "," binding } ")" [ "." NAME ]   (* template(passacaglia, ground=...)  template(passacaglia).ground *)
            | template "." NAME      (* template(deep-house, @tempest_such_stuff).night *)
            | "theme" NAME
            | "phrase" PHRASE
            | GENRE NAME "." NAME ;  (* genre@1 techno.drop; GENRE is genre@N or a bare genre *)
sing        = "sing" ( lyric | PHRASE | NAME ) "on" role ... ;        (* NAME: a phrase a `use phrase` line named *)
template    = "template" "(" NAME "," ( PHRASE | STRING | NAME ) ... ; (* the same *)
```

A source is one word to the parser (`uses::collapse`, as a template reference is: `template::collapse`), in its canonical
spelling: a `gen` call as gen.rs writes it, a template reference with its roles in the form's order, `genre@1` with its version.
`use   theme  ode_to_joy as tune` and `use theme ode_to_joy as tune` are one program. A kind is a word where the source has none
of its own (`theme`, `phrase`, `genre`); a CID, a `gen@N` call and a `template(` speak for themselves. A bare name is no
source: `use ode_to_joy as tune` is refused with "did you mean `use theme ode_to_joy as NAME`?".

### What it lowers to

The `use` stage runs first (compile.rs: use, template, seed, gen, stored), and writes each line out in place:

| source | the name is | the line is written as |
|---|---|---|
| `<cid>.drop`, `gen@N ... section=drop` | a section | `section NAME from <source>`; the stored (and gen) stages write it out |
| `<cid>.drop.bass`, `<cid>.bass`, `gen@N ... part=bass` | a line | `let NAME = <source>`: a stored part's pattern, read in the key it lands in; a seed's part a let a bar (`NAME_*`) |
| `template(F[, PHRASE][, roles]).S` | a section | the template written out alone (`arrange from template(...)`, with the form's own example phrase when it names none), its canonical text known by its CID to the stages after this one (as a `gen` call's program is: `gen::with_generated`), and `section NAME from <cid>.S` |
| `genre@N G.S` | a section | the genre template, from the frozen file `crates/djengine/data/genre-templates.txt`, known by its CID the same way, and `section NAME from <cid>.S` |
| `theme ID` | a line | its bars, `let NAME_1 = ...` ... `let NAME = <NAME_1 ...>`, moved into the program's top-level `key` (`roles::theme_shift`: to its tonic, or its relative key when the theme's mode is the other one; the nearest way) |
| `phrase @ID` | a phrase | nothing; `sing NAME on ROLE` and `template(T, NAME)` are written `@ID` |
| `<cid>`, `gen@N ...` (whole), `template(F, roles)` | a source for a role | nothing; `ROLE=NAME` is written `ROLE=<source>` |

In a role binding the name always stands for its source (a theme's name is the theme id, which the role moves into its own key;
a section's is `<cid>.S`), and a `use` whose name only roles and phrases read writes nothing at its line. A role inside another
`use` line's source takes the names above it the same way (`use template(passacaglia, ground=walk) as episode_form`).

### What it refuses

| it finds | it says, with what would work |
|---|---|
| a source of no kind | `` `foo` is no source for `use` `` (a theme's id: did you mean `use theme foo as NAME`?; a near kind word: did you mean `theme`?) |
| notes, `seed(...)`, a bare `@id` | notes need no `use` (`let riff = d2 ~ a1 ~`); `seed(...)` is no source (`use gen@2 techno seed=42 section=drop as drop`); a phrasebook line is named with `phrase` |
| an unknown theme, phrase, template, template section, genre or genre section | the nearest names and the ones there are |
| `genre@7` | this BeatScript has genre@1, not `genre@7` |
| a stored CID no host has, a section it lacks; a `gen` call that does not resolve | the stored and gen stages' own errors, at the line |
| a name given twice, or the program's own `let`, section or def (or a line's bar `NAME_2`) | `` `tune` is named already, by the `use` on line 1 ``, `` `tune` is bound already: a let `tune` on line 1 `` |
| a name used before its line, or in its own line | `` `tune` is used before its `use` (line 4) ``: move the line above |
| the wrong kind for the place | a section where a part plays a line (name one part of it), a line where `arrange` places a section (play it in a section), a phrase in a role, a source for a role anywhere but after a role's `=`, a name `sing` takes that no `use phrase` line gave |
| a `use` in a block | a `use` goes at the top of the program |
| a template with a phrase and no section | it makes a whole track: name one of its sections |
| a canned section whose parts read the form's own lets in a call (the canon's `chase`, `round` and `sung`, the fugue's `answer`, `counter` and `sung`, the quodlibet's `fate`) | it cannot be borrowed alone, with the form's sections that can, or write the whole form out (`arrange from template(...)`) |
| a name that is one of another line's bars (`tune_1` beside `use theme ... as tune`), or whose bars another name is | bound already, at the second line |

A name never used is a warning (its definition is not written).

### Reports

`check` gives a note at each line saying what the name is; `refs` lists the line as a reference of kind `use` (half `engine` for a
`gen` source, else `structure`; its call the source; its place `section`, `let` or `use`; its name; its pin in `resolved`; for a
`gen` source its generator and what it was fitted to), and `imports` each name with its source (beatscript-api `IMPORTS_DOC`). The MCP
`check`, `expand` and `listen` answer `imports`; `listen` marks a part a `use` made with `from`, and its `name`. Eject writes a line
out under `# from: <source> -> <pin>` (a section through every stage, as its music; a theme as its lets), and a name in a role or a
phrase's place as its source (a line nobody else reads goes), in a later `use` line's source too, so a chain of `use` lines ejects
a line at a time and each step plays the same. `refs::based_on` adds each theme, phrase and genre template (with its
library's or its program's CID) to what a program is based on (its JSON-LD `isBasedOn`, lineage); a stored CID on a `use` line is
credited as any borrow is (`stored::cids_of`), a `gen` call as any call.

### Decisions

- **Written out at the line, as a definition, not substituted at every use.** `arrange intro hook` and `~ tune_* rot 2` need a
  section and lets of those names; substituting a source where a section name goes means nothing. So a section or a line is
  defined once where the `use` stands, and the stages after it see the same `section NAME from <cid>.drop` or `let NAME = ...` a
  person would have typed. In a role the name is the source (a role reads sources, not lets: a theme bound to a role is moved
  into the role's key, which a let of the program's key would not be). This is the design's "written out where it is used", read
  for each place.
- **What the name is follows from the source, and is checked at every use.** The source says section, line, source for a role
  or phrase; a use in the wrong place is an error at it naming what the name is. The places are read from the tokens (an
  `arrange` line or a section line of `arrange { }`, a part line or a mutation, a role's `=`, `sing NAME on`, a template's
  phrase); anywhere else the checker decides as for any name.
- **A theme is moved into the program's key**, as a role moves one (`theme_shift`), and kept as the library has it when the
  program says no key; check says which. A form keeps its own themes in its own key; a `use` is the program's, so it plays in the
  program's.
- **A canned section is a section of a template's written-out program, borrowed by its CID**, the way a `gen` call's section is
  (`gen::with_generated`), so the stored stage writes it out with no second copy of that code: its parts as they play, its `key`
  when it differs, never its control lines (its `sing` and `mix` stay behind, and the stored stage warns). With no phrase it is
  written with the form's own example phrase (`template(passacaglia).ground` is `template(passacaglia, @eccl3_season)`'s ground):
  templates v1 fixes both. These are 6668's "canned/stock sections reachable the same way".
- **The genre templates are frozen as a file, `genre@N`.** `genre_template` builds a genre's starting program from the ontology
  and the instrument catalogue, both of which move; a reference must mean the same music for ever, offline, on every host (beatscript,
  the page's wasm, the editor). So the starters are written once into `crates/djengine/data/genre-templates.txt` (CC0, version 1)
  and compiled in; beatscript-serve's `genre_templates_v1_are_frozen` fails when `genre_template` would now write a genre differently, and
  the answer then is a version 2 beside it, never an edit. The version is in the text (`genre@1`; a bare `genre` is 1 and the
  canonical text writes it), as `gen@N`'s is.
- **A phrase is a name for an `@id`**, read by `sing` and by a template's phrase; the parser resolves `sing NAME on` against the
  `use phrase` lines (a name none gave is an error there, as a `let` gives a name a value only below it), and the canonical text
  keeps the name.
- **Pins**: a stored CID, a `gen` call's program, a canned section's or genre template's program CID; a theme and a phrase are
  pinned by their library (the CID of `themes-pd.txt`, of `phrasebook-pd.txt`; a stock phrase has none); a form by nothing of its
  own (what it writes depends on the role it fills).
- **No new MCP tool.** What can be named is listed by `list_examples` (`imports`: the kinds, the templates and their sections,
  the genre templates and theirs) and `genre_template` (its answer's `use`: each of the genre's sections as a `use` line).
- **The cheat sheet says `use` in one sentence**, after the grammar (the `NAME` paragraph: a phrase's name, `use theme
  ode_to_joy as tune`), and its grammar leaves `use` and `use_source` out, as it left out `use restricted`: the sheet was at its
  30 KB cap before `use` (29.85 KB), and a `use` only names what a program can write in place. BEATSCRIPT.md, this page and the MCP
  `check` description teach the rest. Raising the cap, or trimming the sheet's plan, is Matt's call.
- **One more kind is one more arm** of `uses::Source` (and a word for it in `read_source`): a bar slice of a stored program, a
  render, a set's track would each be one.

### Not done

- **A bar slice of a CID** (`use <cid>[17..24] as NAME`, as `seed(...)[17..24]`): asked for in this run and set aside for Matt's
  yes or no; it would build a `seed::Track` from the compiled program and reuse seed.rs's writer.
- A canned section whose parts read the form's own lets in a call is refused (7 of the 26 sections the templates write); it
  could be borrowed with the lets it reads. A stored program's section of that kind (`section x from <cid>.chase`) has the
  same limit in the stored stage.
- A `use` in an imported snippet is not written out (an `import` brings lets and defs); a `use` names nothing across programs.
- A stored part as a line is its pattern only (as `let riff = <cid>.drop.bass` is); its params stay behind: write them on the
  part line, or borrow the part on the line (`deck A: bass <cid>.drop.bass`).
- A section is not renamed by a `use` (`section verse from hook` is not read); a name is not a parameter of a `def`.
- The editor's highlighting does not mark `theme`, `phrase` and `genre` as words of a `use` line.
- A form named by `use` has no pin of its own (what it writes depends on the role it fills; 6680 item 2 gives nested nodes CIDs).

### Proposals

- `use set <cid>.k as NAME`: a set's k-th track, as a source for a role (sets in a role are built: "Built since" above).
- `use <cid>[a..b] as NAME`, if Matt says yes (above).
