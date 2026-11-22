# Forms: arrange shapes from classical music

A form is a shape for a whole track: which sections there are, the order they come in, and what the arrangement does to them
pass after pass. Classical music named its shapes centuries ago, and they turn out to be BeatScript arrangements: a section
list plus a plan of arrange mutations (`~` from here on, `@` for the next section only). This page names six of them, and a
seventh made for crossing two seeds (`cross`), and gives each a template built from public-domain themes, so
`arrange from template(quodlibet, @psalm42_deep_calleth)` is a whole track, and the written-out program (`expand`) is the form
as plain BeatScript to edit. A form's roles can be filled with other sources: seeds, themes, notes, stored programs, other forms
(see "Roles" below).

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

Write it: `arrange from template(quodlibet, @psalm42_deep_calleth)`. Reference program: **Canon of Fate**.

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

Write it: `arrange from template(canon, @mgm_pease_porridge_hot)`.

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

Every form but the quodlibet names its **roles**, and any of them can be bound to a source after the phrase, `role=source`:

```text
arrange from template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass, upper=ode_to_joy)
arrange from template(rondo, @shanty_drunken_sailor, refrain=gen@1 dubstep seed=54876 part=melody, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876 part=bass))
```

A role nobody binds keeps the form's own theme, so a template with no roles is the track it always was.

| form | roles | what each role is | default | written as |
|---|---|---|---|---|
| passacaglia | `ground`, `upper` | the bass under every section, never changed; the line the variations change | Holst's Mars ostinato; the form's own line | `ostinato`, `upper` |
| canon | `subject`, `ground` | the line chased by copies of itself, a bar and a beat behind; the bass | Pachelbel's violin; Pachelbel's ground | `subject`, `ground` |
| rondo | `refrain`, `episodes` | the tune that comes back; what plays between the returns, a line or sections | Joplin's Entertainer; Bach's Prelude in C | `tune`, `figure` or sections |
| fugue | `subject`, `counter` | the theme entered, answered a fifth up, re-entered; the countersubject drawn toward it | Grieg's Mountain King; Bach's Badinerie | `dux`, `countersubject` |
| theme and variations | `theme` | the one source every variation changes | Dvorak's New World Largo | `air` |
| cross | `a`, `b` | the two sources, each its melody, bass and chords | Ode to Joy over Pachelbel's ground; the Badinerie over a B minor bass | `a_melody` `a_bass` `a_harmony`, `b_...` |

The sources: a **seed** (`gen@1 dubstep seed=54876`, with `section=` and `part=` or without: the form takes the role's part from
the first section that plays it, the drop first, and a whole seed in a role of sections is its sections); a **theme** of the
library by id (`ode_to_joy`, moved into the key the role plays in); **notes**, a bar each (`[d2 ~ a1 ~]`, `<[d2 ~] [g1 ~]>`); a
**stored program**'s part (`<cid>.drop.bass`), section (`<cid>.drop`) or whole (`<cid>`, its sections in its order); and
**another form** (`template(passacaglia, ground=...)`, no phrase), written inside this one under the role's prefix (`ep_ground`),
its plan spread over the role's places. A role is written as a let a bar the sections and the plan read by name, so the plan's
`~ subject_* rot 2` moves whatever fills it; a seed stays a pinned `gen@1` call in the written-out text. Forms nest three deep,
a line is at most 16 bars and a role of sections at most 128 a pass. The whole design is the long note `docs/btw/ext-roles.md`.

## The theme library

The forms' material is the [public-domain theme library](/themes) (`crates/djengine/data/themes-pd.txt`, CC0), the
melodic twin of the [phrasebook](/phrasebook). Each entry is let-ready bars on the 16-step grid in a stated key, with its
role (melody, bass, harmony, arp), work, composer, year (before 1929), mode, meter and the themes it pairs with: those that
share its key or its harmony and morph well with it. A bar of 3/4 is twelve sixteenths (`melody/12`) and a bar of 5/4
twenty (`bass/20`); two bars of 2/4 are one 16-step bar.

- Paste a theme's lets into a program and play it with its part (`melody ode_to_joy`); move it with `transpose(ode_to_joy, 2)`
  or `~ ode_to_joy_* up 1`.
- `genre_template` with `theme` puts one into a genre's program, in the genre's key, on the part of its role.
- `list_examples` with `themes` lists them (by `key`, `role` or `composer`), each with its lets and its part.
- Every entry is written from the melody as the composer wrote it, never from a modern arrangement, edition, transcription or
  recording; its `note` says where a bar is simplified (the Mars triplet is three sixteenths; Danse macabre's entry is the
  interval, not the solo's rhythm). New themes are a change to the file (a new CID), not to the language.

## Reference programs

Three whole hand-written tracks, each the shape its form's template generalises. Each is CC0, carries its sources in a
comment and in linked data beside it (`examples/btw/<name>.jsonld`, `isBasedOn` the public-domain works), and is pinned by
its CID.

| form | program | after | CID |
|---|---|---|---|
| quodlibet | **Canon of Fate** (`canon-of-fate`) | Pachelbel, Canon in D (composed about 1680-1706, first printed 1919); Beethoven, Symphony No. 9 (1824), Ode to Joy; Beethoven, Symphony No. 5 (1808), the fate motif | `z4EBG9j9Z3mPn1vKYrEVc93WXEjYyL7bCGdBM7jX4h8ZWMEuYZx` |
| passacaglia | **Full fathom five** (`full-fathom-five`) | Shakespeare, The Tempest (1623), Ariel's song, sung | `z4EBG9j7BJKjYPfs6QQBQCdjieviWbSwbJqEPm9idg79wTYNLW1` |
| rondo | **Rag Rondo** (`rag-rondo`) | Joplin, The Entertainer (1902) and Maple Leaf Rag (1899; its figure approximated in C, not transcribed) | `z4EBG9izUsbisU2JZtPHUo4hcQpYd8FLyZXLqKUmbtftjj3Lirw` |

## For an agent

1. Pick a form for the shape you want: cross-talk between tunes (quodlibet), slow accumulation (passacaglia), a chase
   (canon), a hook that keeps returning (rondo), voices entering one by one (fugue), one idea seen many ways (theme and
   variations), two seeds crossed (cross).
2. `genre_template` with `genre` and `form` (and a `phrase`, else the form's own): the answer has the program, its `form`
   (shape, plan, themes, reference) and `expanded`, the written-out text to edit.
3. Or write the reference yourself, `arrange from template(<form>, @<phrasebook id>)`, and `expand` it.
4. To make a form your own music, bind its roles: `ground=gen@1 dubstep seed=54876 part=bass`, `subject=ode_to_joy`, notes, a
   stored program's part, another form. `check` says what each role got, in a note at it.
5. For your own form, start from the plan: themes as lets a bar, `drm_*` for the drums that change, the vocal only moved or
   crossed, and `check` and `listen` until it says nothing you did not mean.
