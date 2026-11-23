# Roles: a form's parts bound to sources

Long note for docs/BEATSCRIPT.md "Phrase templates", the roles paragraph (roadmap 6673, 6674, 6678). Code: `crates/djlive/src/roles.rs`
(the role tables, the reader of a binding, the writing out), `crates/djlive/src/template.rs` (the reference and its canonical
spelling), `crates/djlive/templates/cross.beatscript` (the cross form). Tests: `crates/djlive/tests/roles.rs` (no generator: themes,
notes, forms in a role) and `crates/beatscript-api/tests/forms_roles.rs` (seeds in a role, stored programs, eject).

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

## Designed, not built

- **`use SOURCE as NAME`**: a standalone structural import, so a long nested reference reads in lines:
  `use template(passacaglia, ground=gen@1 dubstep seed=54876) as episode_form` then `episodes=episode_form`. `use` exists only as
  `use restricted` today (grammar.ebnf `use`, parse.rs); `use <source> as NAME` would be its second form, a name for a source,
  written out where it is used.

## Built since (docs/beatscript/forms.md "Extensions")

- **Sets in a role**: a stored set's tracks as a role of sections, each track's sections in turn (`episodes=<set cid>`), and one
  track by number (`<cid>.track2`). Its mixes stay behind.
- **A section as a line**: a section where a line goes gives the role its own part of it, a stored program's
  (`subject=<cid>.both`) or one section of another form (`subject=template(cross).both.arp`, the notes it plays the first time
  it plays); the canon's copies are made of its bars.
- **Sub-roles**: `a.bass=...` binds one part of a seed role, beside the whole role or alone.
- **The canon's `delay=` and `interval=`**: the chase's copy N beats behind, N semitones from the leader.
- **Quodlibet roles**: `a`, `b`, `ground`, `third`.
