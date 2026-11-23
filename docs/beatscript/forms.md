# Forms: arrange shapes from classical music

A form is a shape for a whole track: which sections there are, the order they come in, and what the arrangement does to them
pass after pass. Classical music named its shapes centuries ago, and they turn out to be BeatScript arrangements: a section
list plus a plan of arrange mutations (`~` from here on, `@` for the next section only). This page names six of them, and a
seventh made for crossing two seeds (`cross`), and gives each a template built from public-domain themes, so
`arrange from template(quodlibet, @psalm42_deep_calleth)` is a whole track, and the written-out program (`expand`) is the form
as plain BeatScript to edit. A form's roles can be filled with other sources: seeds, themes, notes, stored programs and sets,
other forms and their sections (see "Roles" and "Extensions" below).

It is the found-material principle again ([found phrases](/docs/found-phrases)): borrow the structure, not the recording,
from sources clean by construction. The themes are melodies composed and first published before 1929, written from the
melody itself (the [theme library](/themes)), and the forms are older still. The shapes are starting points, not rules: try a
fugue at 170 or a passacaglia under a gabber kick, and ignore the plan when ignoring it sounds better.

Everything here is one source with many views. The forms are templates (`crates/djlive/src/template.rs`, each with its section
shape, mutation plan and themes); the themes are a CC0 file (`crates/djengine/data/themes-pd.txt`, served at `/themes`,
`/themes.txt` and `/themes.jsonld`, by CID too). The MCP `genre_template` tool takes a `form` (the form at a genre's tempo)
or a `theme` (one theme put into a genre's program, in its key), and `list_examples` with `themes` lists the library. The
ontology has `ns:Form` and a term for each form, linked to the mutation operators it uses and to the found-phrases technique.

## How a form is written

Every form here is built the same way, and so can yours be:

- **Themes are lets, a let a bar.** A theme is its bars (`ode_to_joy_1` ... `ode_to_joy_4`) and their alternation
  (`let ode_to_joy = <ode_to_joy_1 ode_to_joy_2 ode_to_joy_3 ode_to_joy_4>`). A part plays the alternation; a mutation
  changes the bars, with a glob: `~ ode_to_joy_* morph pachelbel_violin_1 0.25`.
- **The drums that change are `drm_*`**, and the kick and the shaker are not, so `~ drm_* grow` and `decay` never take the
  floor away.
- **The phrase is sung in one section** (`sung`, or `voices` in the quodlibet), as in the [found phrases](/docs/found-phrases)
  technique: buried, a little `@ vox_* xfer shk` to cross it onto the shaker's rhythm. The vocal is only ever moved in
  pitch or crossed, never thinned, so no syllable is lost.
- **`~` persists and compounds** across the loop: a form loops two or three times, and each pass hears every change so far.
- **The form keeps its key.** Its themes are written in it; a program's own `key` moves the vocal line and the sources bound to
  its roles, not the themes. `tempo` and `swing` are yours to change.

## Quodlibet

Several themes take turns by section and **cross-morph** toward each other and away, in cycles. A quodlibet was a joke of
the Baroque: popular tunes sung at once, made to fit.

- **Sections:** `canon` (Pachelbel's violin over his ground), `ode` (Beethoven's tune over it, the violin under), `fate`
  (the fate motif in B minor, D major's relative minor), `voices` (the phrase over the ground).
- **Plan:** `~ a_* morph b_1 p` paired later with `~ b_* morph a_1 p`, so each theme leans toward the other; the pair pulled
  away by a morph toward a third line; `@` one-shot borrowings (one section only). The morphs persist and compound pass on pass.
- **Themes:** `pachelbel_ground`, `pachelbel_violin`, `ode_to_joy`, `beethoven_fate`.

```text
arrange {
  canon
  ~ ode_to_joy_* morph pachelbel_violin_1 0.25
  ode
  ~ pachelbel_violin_* morph ode_to_joy_1 0.25
  fate
  @ pachelbel_violin_* morph fate_b_1 0.3
  voices
  ~ ode_to_joy_* morph fate_b_2 0.2
} loop 3 then end
```

Write it: `arrange from template(quodlibet, @psalm42_deep_calleth)`. Reference program: **Canon of Fate**. Its roles are `a`,
`b`, `ground` and `third` ("Extensions"): `template(quodlibet, @psalm42_deep_calleth, a=badinerie, third=mountain_king)`.

## Passacaglia

A **fixed ground** in the bass, never changed, and **cumulative variations** above it: each pass adds a change on top of
the last.

- **Sections:** `ground` (the ground and the chords), `variation` (a line above it, and the drums), `sung` (the phrase
  over the ground).
- **Plan:** `~ upper_* rot 2 | accent 0.25`, `~ drm_hat thin 0.15`, `~ upper_* rev | drift 0.1`, `~ drm_* grow`,
  `~ upper_* up 1`, `~ upper_* down 1`, `~ drm_* decay`, and found-phrase vocals crossed onto the shaker with `@ vox_* xfer
  shk`. Nothing ever touches the ground.
- **Themes:** `mars_ostinato`, Holst's five-beat ostinato, played as `bass/20`: five beats against the bar's four, so the
  ground comes round every five bars and never sits square.

```text
section ground 8 bars {
  deck A: kick kick4 kit=drums | bass/20 mars_ostinato inst=bass
  deck B: harmony chords inst=strings len=8
}
arrange {
  ground variation
  ~ upper_* rot 2 | accent 0.25
  sung
  ~ upper_* rev | drift 0.1
  variation
} loop 3 then end
```

Write it: `arrange from template(passacaglia, @eccl3_season)`. Reference program: **Full fathom five**.

## Canon

A theme **chasing its own delayed or rotated copy**. Pachelbel's three violins play one line two bars apart; a round is
a canon anyone can sing.

- **Sections:** `lead` (the theme alone), `chase` (a copy a bar behind, an octave down), `round` (and a third a beat
  behind), `sung`.
- **Plan:** the copies are made of the theme's own bars (`let follow = <pachelbel_violin_2 pachelbel_violin_1>`,
  `let close = rotate(pachelbel_violin, 4)`), so whatever the arrangement does to the theme (`~ pachelbel_violin_* rot 2`,
  `rev | accent 0.25`) its followers do too, and the chase stays exact.
- **Themes:** `pachelbel_ground`, `pachelbel_violin`.

```text
let follow = <pachelbel_violin_2 pachelbel_violin_1>
let close = rotate(pachelbel_violin, 4)
section round 8 bars {
  deck B: melody pachelbel_violin | arp octave(follow, -1) | harmony close
}
```

Write it: `arrange from template(canon, @mgm_pease_porridge_hot)`. How far behind and at what interval the chase's copy is are
the canon's numbers, `delay=` (beats) and `interval=` (semitones): `template(canon, @mgm_pease_porridge_hot, delay=6,
interval=7)` is a canon at the fifth, a bar and two beats behind ("Extensions").

## Rondo

A **refrain that keeps coming back**, changed each time, between contrasting sections: A B A C A.

- **Sections:** `refrain` (A: Joplin's Entertainer over a stride), `episode` (B: Bach's Prelude in C), `sung` (C: the
  phrase over the prelude's figure).
- **Plan:** the refrain is mutated on each return: `~ the_entertainer_* rot 2 | accent 0.3`, then `@ the_entertainer_*
  mirror | vel 2` for one return only, `~ up 1` and later `~ down 1`; the episode morphs toward the refrain
  (`~ bach_prelude_c_* morph the_entertainer_1 0.2`).
- **Themes:** `the_entertainer`, `bach_prelude_c`.

```text
arrange {
  refrain episode
  ~ the_entertainer_* rot 2 | accent 0.3
  refrain sung
  @ the_entertainer_* mirror | vel 2
  refrain
} loop 2 then end
```

Write it: `arrange from template(rondo, @shanty_drunken_sailor)`. Reference program: **Rag Rondo**.

## Fugue

A **subject** that enters, is **answered** a fifth up, and **re-enters transposed** while the other lines **morph toward
it**.

- **Sections:** `subject` (Grieg's Mountain King alone), `answer` (and again a fifth up: `transpose(mountain_king, 7)`),
  `counter` (Bach's Badinerie against both, the subject in the bass), `sung`.
- **Plan:** `~ mountain_king_* down 2`, later `~ mountain_king_* up 2` (the subject and its answer move together, being one
  theme); `~ badinerie_* morph mountain_king_1 0.25` then `morph mountain_king_3 0.2`, the countersubject drawn toward it.
- **Themes:** `mountain_king`, `badinerie`.

```text
section answer 8 bars {
  deck B: melody mountain_king | arp transpose(mountain_king, 7)
}
arrange {
  subject answer
  ~ badinerie_* morph mountain_king_1 0.25
  counter
  ~ mountain_king_* down 2
  sung
} loop 3 then end
```

Write it: `arrange from template(fugue, @tyger_burning_bright)`.

## Theme and variations

**One source, a different transform each section**, and no cross-morphs: the theme is heard plain, then changed one way,
then another, and comes back.

- **Sections:** `theme`, then `variation` again and again, `sung`, and `theme` to end.
- **Plan:** each variation is a one-shot `@`, so the theme reverts after it: `@ new_world_largo_* rev`, `invert`,
  `stutter 2`, `oct -1`, `half | accent 0.3`. Nothing is morphed toward anything else.
- **Themes:** `new_world_largo`.

```text
arrange {
  theme
  @ new_world_largo_* rev
  variation
  @ new_world_largo_* invert
  variation
  @ new_world_largo_* stutter 2
  variation
  theme
} loop 2 then end
```

Write it: `arrange from template(theme-and-variations, @dickinson_chariot_death)`.

## Cross

Two sources **take turns, overlap, and morph toward each other and back**: a quodlibet of two, made for crossing two seeds of
the generator, in any genres (`a=gen@1 dubstep seed=54876, b=gen@1 house seed=7`).

- **Sections:** `a_leads` (a's tune over its own bass and chords), `b_leads`, `both` (b's tune over a's bass, a's tune under it
  as an arp), `sung` (the phrase over the crossing).
- **Plan:** `~ a_melody_* morph b_melody_1 0.25` paired with `~ b_bass_* morph a_bass_1 0.25`, an `@ a_melody_* xfer b_melody_1`
  one-shot in the overlap, then the pull turned round (`~ b_melody_* morph a_melody_1 0.25`, `~ a_bass_* morph b_bass_1 0.2`).
  The morphs persist and compound pass on pass (loop 3).
- **Themes** (unbound): `ode_to_joy` over `pachelbel_ground`, and `badinerie` over a bass in B minor, D major's relative minor.

Write it: `arrange from template(cross, @psalm42_deep_calleth, a=gen@1 dubstep seed=54876, b=gen@1 house seed=7)`. Example:
**cross two seeds** (`cross-two-seeds`).

## Roles: the form is the structure, sources fill it

Every form names its **roles**, and any of them can be bound to a source after the phrase, `role=source`:

```text
arrange from template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass, upper=ode_to_joy)
arrange from template(rondo, @shanty_drunken_sailor, refrain=gen@1 dubstep seed=54876 part=melody, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876 part=bass))
```

A role nobody binds keeps the form's own theme, so a template with no roles is the track it always was.

| form | roles | what each role is | default | written as |
|---|---|---|---|---|
| quodlibet | `a`, `b`, `ground`, `third` | the first tune; the second, drawn toward the first; the bass; the third line both are pulled toward | Pachelbel's violin; Ode to Joy; Pachelbel's ground; the fate motif in B minor | `tune_a`, `tune_b`, `ground`, `third` |
| passacaglia | `ground`, `upper` | the bass under every section, never changed; the line the variations change | Holst's Mars ostinato; the form's own line | `ostinato`, `upper` |
| canon | `subject`, `ground` (and the numbers `delay`, `interval`) | the line chased by copies of itself, a bar and a beat behind; the bass | Pachelbel's violin; Pachelbel's ground | `subject`, `ground` |
| rondo | `refrain`, `episodes` | the tune that comes back; what plays between the returns, a line or sections | Joplin's Entertainer; Bach's Prelude in C | `tune`, `figure` or sections |
| fugue | `subject`, `counter` | the theme entered, answered a fifth up, re-entered; the countersubject drawn toward it | Grieg's Mountain King; Bach's Badinerie | `dux`, `countersubject` |
| theme and variations | `theme` | the one source every variation changes | Dvorak's New World Largo | `air` |
| cross | `a`, `b` (and their parts alone, `a.bass` ...) | the two sources, each its melody, bass and chords | Ode to Joy over Pachelbel's ground; the Badinerie over a B minor bass | `a_melody` `a_bass` `a_harmony`, `b_...` |

The sources: a **seed** (`gen@1 dubstep seed=54876`, with `section=` and `part=` or without: the form takes the role's part from
the first section that plays it, the drop first, and a whole seed in a role of sections is its sections); a **theme** of the
library by id (`ode_to_joy`, moved into the key the role plays in); **notes**, a bar each (`[d2 ~ a1 ~]`, `<[d2 ~] [g1 ~]>`); a
**stored program**'s part (`<cid>.drop.bass`), section (`<cid>.drop`) or whole (`<cid>`, its sections in its order); a **stored
set** (`<cid>` of a `set { play ... }`: its tracks in order) or one of its tracks (`<cid>.track2`); **another form**
(`template(passacaglia, ground=...)`, no phrase), written inside this one under the role's prefix (`ep_ground`), its plan spread
over the role's places; and **one section of another form** (`template(cross).both`, `.both.arp`). A role is written as a let
a bar the sections and the plan read by name, so the plan's `~ subject_* rot 2` moves whatever fills it; a seed stays a pinned
`gen@1` call in the written-out text. Forms nest three deep, a line is at most 16 bars and a role of sections at most 128 a pass.
The whole design is the long note `docs/beatscript/ext-roles.md`.

## Extensions: sets, sections, parts, numbers

Five ways to go further with a form's roles, each with a default that keeps every reference that does not say it written out
as it was (the pins of `roles.rs bound_forms_are_pinned` and `template.rs templates_v1_are_pinned`, and the examples' pins). Code: `crates/djlive/src/roles.rs`;
tests: `crates/djlive/tests/form_extensions.rs` (no generator) and `crates/beatscript-api/tests/forms_roles.rs` (sets, stored programs,
seeds). Example: **Canon of the crossing** (`canon-of-the-crossing`).

### Grammar

```ebnf
binding     = NAME [ "." role ] "=" ( source | [ "-" ] INT ) ;
source      = gen | NAME | notes | form | stored | track ;
form        = "template" "(" NAME { "," binding } ")" [ "." NAME [ "." role ] ] ;
track       = CID "." NAME [ "." NAME ] [ "." role ] ;
```

The canonical spelling puts each role's parts right after the role (in the order melody, bass, harmony) and a form's numbers
after its roles: `template(canon, @x, subject=..., delay=6, interval=7)`, `template(cross, @x, a=..., a.bass=..., b=...)`.

### A stored set in a role

A set (`set { play ... }`, docs/BEATSCRIPT.md "Sets") is a program by CID like any other, so a role that takes sections takes one: the
set's **tracks, in order, each its sections in the order it plays them**, one track a place. The rondo's two episodes a pass
are the set's first track and its second:

```text
arrange from template(rondo, @shanty_drunken_sailor, episodes=<set cid>)
```

is written out as `section ep_t1_verse from <track 1>.verse`, `section ep_t1_hook from <track 1>.hook`, `section ep_t2_intro
from <track 2>.intro` ..., each under its doc comment, the arrangement's `episode` places taking `ep_t1_verse ep_t1_hook
ep_t1_verse` and then `ep_t2_intro ep_t2_lift`. With more tracks than places they are shared out in order; with fewer they come
round. **`.trackN`** names one track, and is that track's program from then on: `<cid>.track2` (its sections), `<cid>.track2.lift`
(a section), `<cid>.track2.lift.melody` (a part, a line). A track that is itself a set is its tracks in turn, named by their
place (`ep_t2_1_drop`: the drop of track 1 of track 2). After a program that is no set, `.track2` is its section called `track2`,
as it always was.

- **Lowers to** the tracks' sections borrowed by their own CIDs (stored.rs writes them out); the set's text goes with the program
  for a link to carry (`plays`), since the written-out text names only its tracks.
- **Refuses**, at the reference: a set where a line goes (`refrain=<set>`: "one part of one of its tracks",
  `refrain=<set>.track1.SECTION.melody`); a set's section (`<set>.verse`: "it has tracks, not sections", `<set>.track1.verse`);
  a track it does not have ("the set has 2 tracks: `.track3` is none of them", `track1 to track2`; `.track0` or `.trackx`: tracks
  count from 1); `.trackN` after a program that is no set and has no section of that name; a track with no sections; more than
  128 bars a pass (the cap every role of sections has; for a set the help names its first track, `<cid>.track1`).

### A section where a line goes

A section is a source where a line is expected: the role takes **its own part of it** (a subject's is the melody, a ground's the
bass, the episodes' line the arp), or the part it names. This was already so for a seed (`gen@1 dubstep seed=54876
section=drop` fills in the part); now it is so for a stored program's section (`subject=<cid>.both` is `<cid>.both.melody`) and
for **one section of another form**, written after its `)`:

```text
arrange from template(canon, @mgm_pease_porridge_hot, subject=template(cross).both.arp, delay=6, interval=7)
```

is a canon whose subject is the cross's overlap: the arp of its `both` section, which is Ode to Joy as the cross's plan has
moved it by then (morphed a quarter of the way toward the Badinerie; the plan's one-shot `xfer` falls on the second `both`),
chased by copies of itself.

- **Lowers to** notes: the inner form is written out on its own and compiled, in its own key and at its own tempo, and the part is
  written as the notes it plays the first time the section plays, a let a bar (`let subject_1 = ...`), then moved from the inner
  form's key into the role's (the rule a theme is moved by: to the tonic, or to the relative key when the modes differ). The outer plan's mutations then read it by
  name, and the canon's copies are made of its bars. In a seed role (`a=template(canon).round`, `a=<cid>.verse`) a section gives
  each of the role's parts. Where a role takes sections, `template(cross).both` is that section, its parts written out as a stored
  program's section is, standing in for the role's section (`episode`) in its own key; `template(cross).both.arp` there is a line.
- **Refuses**: a section the form does not have (with the nearest, and its sections; its sung section is the outer form's phrase,
  not one of its sections here); a part the section does not play ("the both of template(cross) plays no snare; it plays ...");
  a part on both decks; a part with a cycle of its own; a line of more than 16 bars; a part before a section (`.arp` alone).

### Parts of a role (`a.bass=`)

A seed role (the cross's `a` and `b`: melody, bass, chords) has its parts bound one at a time. Alone, `a.bass=` replaces the
default's bass and leaves its tune and chords; with the whole role bound, the part bound alone wins over what the whole gives it:

```text
arrange from template(cross, @psalm42_deep_calleth, a=gen@1 dubstep seed=54876, a.bass=<[d2 ~ ~ ~ a1 ~ ~ ~] [g1 ~ ~ ~ a1 ~ ~ ~]>)
```

is the seed's melody and chords over a hand-written bass; the plan's `~ a_bass_* morph b_bass_1 0.2` reads the bound bass.

- **Lowers to** the part's let (`a_bass`) under `# a.bass, bound: ...`; the header says `a: gen@1 ..., a.bass=<...>`.
- **Refuses**: a part the role does not have (`a.bas`: "did you mean `a.bass`?"); a part of a role that is one line
  (`ground.bass=`: "`ground` is one line"); a part bound twice, by `a.bass=` twice or by `a=... part=bass` and `a.bass=`; a whole
  role all of whose parts are bound alone ("`a=` gives nothing").

### The canon's `delay=` and `interval=`

The chase's copy (`follow`, played by the `chase`, `round` and `sung` sections as the arp) is a bar behind and an octave down. Two
numbers move it:

- **`delay=N`**, beats behind the leader, 1 to 4k - 1 for a subject of k bars: its bars come round N / 4 bars late (the last
  first), and the beats left over move each bar along (`rotate`, wrapping in the bar, as the canon's own `close` copy is "a
  beat behind"). `delay=6` on a two-bar subject is `<rotate(pachelbel_violin_2, 8) rotate(pachelbel_violin_1, 8)>`; on a one-bar
  subject `delay=3` is `rotate(subject, 12)`.
- **`interval=N`**, semitones from the leader, -24 to 24: `-12` is `octave(follow, -1)` as before, `-24` `octave(follow, -2)`, `0`
  the copy itself, anything else `transpose(follow, N)` (`7` a fifth up: a canon at the fifth, the fugue's answer's way).

The comments say the delay and the interval in words (`# chase: the copy a bar and 2 beats behind, a fifth up`), the header gives
each number given, and `check` says each in a note. The round's third voice (`close`) stays a beat behind, in unison.
**Refuses**: a delay of the subject's length or more ("1 to 7 beats for a subject of 2 bars: 8 beats is the subject itself, in
unison"); an interval past two octaves; a number that is not whole (`delay=2.5`); a source for a number (`delay=ode_to_joy`) or a
number for a role (`subject=3`); a number another form does not take (`delay` is no role of the rondo).

### The quodlibet's roles

The quodlibet names its roles as the other forms do: **`a`** (the first tune, Pachelbel's violin, written `tune_a`), **`b`** (the
second, Ode to Joy, `tune_b`), **`ground`** (Pachelbel's ground) and **`third`** (the line the other two are pulled toward, the fate
motif a semitone down in B minor). Its plan's cross-morphs read them (`~ tune_b_* morph tune_a_1 0.25`, `@ tune_a_* morph
third_1 0.3`); binding `third` also takes away the fate motif's own lets, which only the default was made of.

```text
arrange from template(quodlibet, @psalm42_deep_calleth, a=badinerie, b=new_world_largo, third=mountain_king)
```

### Decisions

- **A set is its tracks, not its mix.** A set's mixes overlap two tracks on two decks bar by bar; in a form the tracks are
  separated by the form's own sections (the refrains), so there is nothing to overlap. The mixes and the `until` cuts stay
  behind, and the note says so (`its mixes and cuts stay behind`), as a borrowed section's control lines do.
- **A playlist is not a source.** A playlist is an account's list, changed in place and named by an id; a role's source is pinned
  by what it is. A set is the content-addressed program of programs, so a set by CID is what a role takes.
- **`.trackN`, not `.N`.** `<cid>.2` would need the lexer to read a `.` before a digit, which it reads as no token today (a lone
  `.`); `track2` is a word. After a CID that is no set, `.track2` is the program's section called `track2` (a section may be
  named so), read and written out exactly as before sets were sources; only a program with no such section is told it is no set.
- **A set's nested tracks are named by their place** (`ep_t2_1_drop`: track 1 of track 2), the way `.trackN` counts them; a track the set
  plays twice is written once, under its first place.
- **A set is held to the same 128 bars a pass** as any role of sections, so a written-out form stays bounded; a set of longer
  tracks binds one track at a time.
- **A section where a line goes is its part as it first plays**, the same first stretch a stored program's section is
  (`stored.rs`), so the cross's `both` is the overlap as heard (the plan's morphs before it in), and the section as a source is
  notes, structure a person could type: the inner form's later mutations do not follow it, the outer plan's do.
- **A form's section is moved by the theme rule**, and a form's section where sections go keeps its own key with a `key` line,
  as an inner form's sections do.
- **The form a section is taken from is written out on its own, as a function of the reference alone**: in its own key (no
  section of it says another) and at its own tempo, and compiled; its plan's dice are that program's (mutations seed from a
  program's CID). So the program's tempo, swing or comments leave the notes as they are, and its key moves them by the key rule
  and nothing else (`key e` is the D notes up a whole tone). The one exception: when a role of the form is bound to a seed, the
  form is written at the program's tempo, because a seed is generated at the program's tempo everywhere. What the inner form's
  own bindings say in `check` is left out: it names lets the program does not have.
- **A bar of one step the plan morphs or crosses toward is written in brackets** (`let a_bass_1 = [d2]`, as a seed's bars are):
  a bare `d2` is no pattern `morph` or `xfer` takes, so `a.bass=[d2]` failed at the plan. Only there: anywhere else the bare step
  checked, and its text stays as it was, because the written-out text's CID seeds the plan's dice (a bracket elsewhere would
  redraw the drums).
- **A stored subject is one let** (`let subject_1 = <cid>.verse.melody`, which the stored stage writes out), however many bars it
  has, so the canon's copy of it moves within the bar: `delay=` is 1 to 3 beats there, and the refusal says so. A subject of
  notes, a theme or a seed is a let a bar, and takes a delay of bars.
- **The part a section gives is the role's own** (the same part a whole seed gives it), so `subject=<cid>.both` and
  `subject=gen@1 ... section=both` mean the same thing; another part is named, as a stored program's is.
- **A part bound alone wins over the whole.** `a=seed, a.bass=notes` is the reason to have parts: the seed for its tune and
  chords, your bass. Only two bindings of the very same part are an error.
- **`delay` counts beats** (the canon's copies are "a bar and a beat behind": both in beats), and its beats wrap in the bar
  rather than carrying across the bar line, because the copy stays made of the subject's bars: `~ subject_* rot 2` moves the
  leader and the copy together, which a copy cut and pasted across bar lines would not.
- **`interval` counts semitones**, as `transpose` and the fugue's answer (`transpose(mountain_king, 7)`) do; a diatonic interval
  would keep a canon at the fifth in key, but no part of the language moves by degrees yet.
- **The numbers move the chase's copy only.** The round's third voice is the stretto of the form (a beat behind, in unison); a
  number for it would be a third parameter nobody asked for.
- **A number given as its default is said**: `delay=4` writes the same copy as no delay, but the reference is other text and so
  another CID, as `vel=` said at its default is.
- **The quodlibet's lines are `tune_a` and `tune_b`**, not `a` and `b`: a let named `a` would read as a note.

### Not done

- Lineage (`refs.rs based_on`, which reads the text alone) names a set bound in a role but not its tracks: only a check that has the
  set knows them. The store credits them (it credits what the check drew on: `plays`), and `stored.rs cids_of` reads every CID
  a role names (a bare one, a set's `.track2.drop.bass`, one in a form in a role).
- A whole stored program in a role whose arrangement mutates a section borrows `drop@2` by that name (a name `section ... from`
  cannot read); a set's tracks borrow the section (`drop`) instead. The same fix in `borrowed` is a one-liner left for its own
  change.
- A set's tracks that are flat programs (no sections) are refused, not borrowed whole.

### Proposals

- A diatonic `interval=` (`interval=5th`, by degrees of the key) once the language moves notes by degrees.
- A delay for the round's third voice (`stretto=`), if a canon of four voices is wanted.
- `.N` for a set's track, if the lexer is taught a `.` before a digit after a CID.

A source can be named first, on a line of its own, and the name bound to the role (`use`, docs/BEATSCRIPT.md "Name a source"), so a
nested reference reads in lines:

```text
use gen@1 dubstep seed=54876 section=drop part=bass as walk
use theme the_entertainer as tune
use template(passacaglia, ground=walk) as episode_form
arrange from template(rondo, @shanty_drunken_sailor, refrain=tune, episodes=episode_form)
```

In a role the name is its source, exactly as if written there (the theme is moved into the role's key by the role). And a form's
sections can be borrowed one at a time into a program of your own, as **canned sections**: `use template(passacaglia).ground as
ground` is the passacaglia's ground (the Mars ostinato under its chords, written with the form's own example phrase), and
`arrange intro ground` places it like a section of yours (`list_examples` with `imports` lists every template's sections).

## The theme library

The forms' material is the [public-domain theme library](/themes) (`crates/djengine/data/themes-pd.txt`, CC0), the
melodic twin of the [phrasebook](/phrasebook). Each entry is let-ready bars on the 16-step grid in a stated key, with its
role (melody, bass, harmony, arp), work, composer, year (before 1929), mode, meter and the themes it pairs with: those that
share its key or its harmony and morph well with it. A bar of 3/4 is twelve sixteenths (`melody/12`) and a bar of 5/4
twenty (`bass/20`); two bars of 2/4 are one 16-step bar.

- Paste a theme's lets into a program and play it with its part (`melody ode_to_joy`); move it with `transpose(ode_to_joy, 2)`
  or `~ ode_to_joy_* up 1`.
- `genre_template` with `theme` puts one into a genre's program, in the genre's key, on the part of its role.
- `use theme ode_to_joy as tune` names one in a program of your own: its lets named for the name (`tune_1 ... tune`), moved into the program's
  `key` (to the relative key when the theme's mode is the other one), so `deck B: melody tune` plays it and `~ tune_* rot 2`
  changes it.
- `list_examples` with `themes` lists them (by `key`, `role` or `composer`), each with its lets and its part.
- Every entry is written from the melody as the composer wrote it, never from a modern arrangement, edition, transcription or
  recording; its `note` says where a bar is simplified (the Mars triplet is three sixteenths; Danse macabre's entry is the
  interval, not the solo's rhythm). New themes are a change to the file (a new CID), not to the language.

## Reference programs

Three whole hand-written tracks, each the shape its form's template generalises. Each is CC0, carries its sources in a
comment and in linked data beside it (`examples/beatscript/<name>.jsonld`, `isBasedOn` the public-domain works), and is pinned by
its CID.

| form | program | after | CID |
|---|---|---|---|
| quodlibet | **Canon of Fate** (`canon-of-fate`) | Pachelbel, Canon in D (composed about 1680-1706, first printed 1919); Beethoven, Symphony No. 9 (1824), Ode to Joy; Beethoven, Symphony No. 5 (1808), the fate motif | `z4EBG9j86dPTC9HH6EjtBJFAWZsJLiPum4np9zoJQC9PzWm9SoC` |
| passacaglia | **Full fathom five** (`full-fathom-five`) | Shakespeare, The Tempest (1623), Ariel's song, sung | `z4EBG9j3xZiLH5Sa8EnFW5sg2WuSPRuMbBaTr3JHeQjfWWE7Ug9` |
| rondo | **Rag Rondo** (`rag-rondo`) | Joplin, The Entertainer (1902) and Maple Leaf Rag (1899; its figure approximated in C, not transcribed) | `z4EBG9izzm3asNWWapjXtPFSZoPzgyj4cKjPELrw4jvrhdjPxrW` |

## For an agent

1. Pick a form for the shape you want: cross-talk between tunes (quodlibet), slow accumulation (passacaglia), a chase
   (canon), a hook that keeps returning (rondo), voices entering one by one (fugue), one idea seen many ways (theme and
   variations), two seeds crossed (cross).
2. `genre_template` with `genre` and `form` (and a `phrase`, else the form's own): the answer has the program, its `form`
   (shape, plan, themes, reference) and `expanded`, the written-out text to edit.
3. Or write the reference yourself, `arrange from template(<form>, @<phrasebook id>)`, and `expand` it.
4. To make a form your own music, bind its roles: `ground=gen@1 dubstep seed=54876 part=bass`, `subject=ode_to_joy`, notes, a
   stored program's part, a stored set, another form or one of its sections (`template(cross).both.arp`), one part of a seed
   role (`a.bass=[d2 ~ a1 ~]`), the canon's `delay=` and `interval=`. `check` says what each role got, in a note at it.
5. For your own form, start from the plan: themes as lets a bar, `drm_*` for the drums that change, the vocal only moved or
   crossed, and `check` and `listen` until it says nothing you did not mean.
