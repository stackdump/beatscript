# examples/beatscript

One program per BeatScript extension feature, a baseline, and `showcase.beatscript`, a whole track that uses them. The language is in
`docs/BEATSCRIPT.md` (each feature has a short section under "The language extensions" and a long note, `docs/beatscript/ext-<feature>.md`);
`docs/BEATSCRIPT-EXTENSIONS.md` is the contract they were built to, kept as history.

Every `.beatscript` here is checked by `crates/djlive/tests/examples_beatscript.rs`: it compiles, its canonical text is a fixed point, and 40 random
reformattings keep its CID. All of them are original and CC0-1.0. A file that needs a host (a registry, a saved snippet) says
`#! skip-test: <why>` in its header block (the leading `#!` lines, usually the first) and is tested where the host is.

| file | shows | BEATSCRIPT.md section | long note |
|---|---|---|---|
| `baseline.beatscript` | the language before the extensions: a small song of a Euclid kick, a snare, hi-hats, a bass and a chord | Patterns | |
| `two-beds-and-a-theme.beatscript` | two seeds of the seeded generator (`gen@2 ... section=drop part=bass`) fitted to one key and tempo, layered on the two decks, crossed with `morph` and `xfer`, and Beethoven's Ode to Joy (a public-domain theme) over them | Call the generator | `ext-ffi.md` |
| `vendored-beds.beatscript` | the same kind of beds ejected: two `gen@2` lets written out as notes, each under its `# from: <call> -> <cid>` line (a comment: the CID is the notes'); saved on the site, the lines are kept as `meta.based_on`, its JSON-LD's `isBasedOn` lists them as `vendored` and lineage counts it in `vendoredFrom` | Call the generator (Eject) | `ext-ffi.md` |
| `arith.beatscript` | arithmetic in every number, `let` folding, a def's arithmetic argument, `when` guards in a `repeat` and a def | Arithmetic and guards | `ext-arith.md` |
| `timing.beatscript` | `at bar 1 beat 2`, `over 2 beats`, `for 3 beats`, `every 1/2 bar from 1`, `before drop 1/2 bar` | Time finer than a bar | `ext-timing.md` |
| `tempo.beatscript` | a tempo that ramps (`100 -> 128 over 8 bars s-curve`), a tempo in a section, a timed `at bar 12: tempo ...` | Tempo curves | `ext-tempo.md` |
| `shapes.beatscript` | `exp` `log` `s-curve` `step N` ramps and `lfo` values (`sine`, `tri`, `saw`) in control and `do` lines | Shapes and lfo | `ext-shape.md` |
| `scale.beatscript` | `deg( )` with accidentals and octave marks, Roman numerals with inversions, `key cm` in a section, `voice-lead` | Degrees, numerals, a key a section | `ext-scale.md` |
| `progression.beatscript` | the chords written once as Roman numerals (`<i VI III VII>`, `arp(VI, up, 8)`), and a section's `key cm` that moves the pad, bass, arp and melody together | Degrees, numerals, a key a section | `ext-scale.md` |
| `euclid.beatscript` | `e(k, n, rot)` and parts with a cycle of their own (`hihat/12`, `melody/10`): a polymeter and its period | Euclidean rhythms and polymeter | `ext-meter.md` |
| `chance.beatscript` | `x*16?0.45`, `choose [..] weights 4 2 1 seed=4`, `maybe([..], 0.8, 4)`: parts that differ each bar and are a function of the text | Seeded chance | `ext-chance.md` |
| `algebra.beatscript` | `stack(...)`, `then(...)`, `silence`, a `let` that names a composition, `arrange intro stack(then(riff, riff), verse) coda` | Composition: stack and then | `ext-algebra.md` |
| `ports.beatscript` | `def` with `in:` and `out:` ports, `emit`, `wire a.out -> b.chords`: checked glue | Typed ports | `ext-ports.md` |
| `ports-import.beatscript` | the same across an `import`: a saved snippet keeps its defs with their ports and its wires (`skip-test`: tested with a fake store in `crates/djlive/tests/ext_ports.rs`) | Typed ports | `ext-ports.md` |
| `macros.beatscript` | `macro riser on A over 8 bars`, `macro ping-pong`, `macro breakdown`, `macro cathedral`, `macro tape_stop`: the DJ macro catalogue by name on the control deck, each starting on the bar its line names (`render` and `listen` play them) | Macros: the DJ catalogue | `ext-control.md` |
| `mix.beatscript` | `duck B under A.kick 6db attack=5ms release=120ms` and a `mix { ... }` that is put back where its section ends | Mix moves: duck and mix | `ext-mix.md` |
| `sing.beatscript` | `sing "words" on melody voice=alto`: typed lyrics on a part's own notes (a neural voice falls back to the formant voice and `check` says so) | Sung lyrics | `ext-sing.md` |
| `linked.beatscript` | the `#! genre:` and `#! tags:` header lines that link a program to the bitwrap ontology, and the Euclidean rhythms `btw jsonld` links | Links to the ontology | `ext-linked.md` |
| `assert.beatscript` | `assert ends`, `length`, `key`, `period` (proved by `check`) and `level` (measured by `btw test`) | Assertions and `btw test` | `ext-assert.md` |
| `constraints.beatscript` | `assert never A.snare beside A.snare`, `assert never B during sing in verse`, `assert bass ends on 1`: rules about the notes, decided by `check` (the bar and the step when one is broken) | Assertions and `btw test` | `ext-assert.md` |
| `perform.beatscript` | `knob(3)`, `pad(5)`, `fader(A)`, `held(key)`, `held(c4)` in control values and `when` guards, read when their line is due and logged into a performance | Performed inputs | `ext-perform.md` |
| `follow.beatscript` | `master: filter lp knob(1)`: a value line follows its knob on every bar, an input written alone covers the filter's range, `every 1/16 bar:` reads when it is due, a knob nothing has moved rests at 1/2 | Performed inputs | `ext-perform.md` |
| `launch.beatscript` | `launch drop on pad(2) next phrase 16`: three scenes (sections) on three pads, brought in on a phrase line; a host with no pad plays the written song | Scenes and launch | `ext-perform.md` |
| `deep-house-60.beatscript` | one hour of hand-written deep house: 1824 bars of chained nets with a coprime `evolve` clock per section (see below) | Evolve | |
| `answer.beatscript` | `rests(call)` and `rests(call, reply)`: a clap and a hat that play only the steps the kick leaves empty, from one `let` the two decks share | Call and response | `ext-answer.md` |
| `arrange-every-n.beatscript` | `~3 riff rev`, `~2 bass_* drift 0.2` and `@4 kik double` in a looping `arrange { }`: a mutation that fires only on every Nth pass (`check` lists the passes it fired on) | Arrange mutations | |
| `control-per-pass.beatscript` | `~2 control off`, `~2 control duck on`, `@3 control freeze off`, `~ control on` in a looping `arrange { }`: the control deck switched pass by pass, all of it or one call's lines (`check` lists what each play holds back) | Arrange mutations, The control deck | `ext-control.md` "Per pass: the switch" |
| `seed.beatscript` | `seed(techno, 42)`: a section from a generated track's bars, its parts as patterns (and a `let`), re-voiced and transformed: start from a seed, then iterate | Start from a seed | |
| `swung-seed.beatscript` | `arrange from seed@2(jazz, 7)`: seeds v2, the whole track with its genre's swing (`swing 0.6`, which seeds v1 never write), at the program's own tempo | Start from a seed | `ext-ffi.md` |
| `arrange-jux.beatscript` | `~ riff jux rev`, `~ riff jux rot 3 \| thin 0.2`, `~ hook rot 1 \| jux rev \| up 2`: a pattern keeps its left copy and plays a mutated copy hard right (best on headphones) | Arrange mutations | `docs/ARRANGE.md` "Stereo" |
| `arrange-in-eight-lines.beatscript` | the smallest arrange that moves: one section played eight times, `~` for good and `@` for one play, and three comments that say so (also beatscript.app's example of the name) | Arrange mutations | |
| `such-stuff-as-dreams-122.beatscript` | `template(deep-house, @tempest_such_stuff)`: a phrasebook line compiled to a track; its syllables set the vocal line's length, its stress the accents and the beats, its meter (iambic: the bar) the subdivision | Phrase templates | `ext-template.md` |
| `assyrian-122.beatscript` | the same template on an anapestic line (`@byron_assyrian_wolf`): the triplet feel, the voice and the shaker in a cycle of twelve steps | Phrase templates | `ext-template.md` |
| `commit-message-122.beatscript` | the same template on any words: a commit message, its stress worked out (small words weak, the first syllable of the rest strong) | Phrase templates | `ext-template.md` |
| `canon-of-fate.beatscript` | the QUODLIBET form's reference program: themes after Pachelbel's Canon, Beethoven's Ode to Joy and fate motif take turns and cross-morph (`~ a morph b p` paired with `~ b morph a p`); its sources in `canon-of-fate.jsonld` | Phrase templates | `forms.md` |
| `full-fathom-five.beatscript` | the PASSACAGLIA form's reference program: a fixed ground under cumulative variations, singing Ariel's song (The Tempest); `check` warns of two words the lexicon lacks, pinned as written | Phrase templates | `forms.md` |
| `rag-rondo.beatscript` | the RONDO form's reference program: a Joplin refrain returning mutated between contrasting strains | Phrase templates | `forms.md` |
| `cross-two-seeds.beatscript` | `template(cross, ..., a=gen@1 dubstep seed=54876, b=gen@1 house seed=7)`: two seeds in different keys of their own, fitted to the program's and crossed: a leads, b leads, both, the morphs pulling one toward the other and back (`skip-test`: needs the generator; tested in `crates/beatscript-api/tests/forms_roles.rs`) | Phrase templates (roles) | `ext-roles.md` |
| `rondo-of-passacaglias.beatscript` | one seed in every role of a nested form: a rondo whose refrain is the seed's melody and whose episodes are a passacaglia over its bass (`skip-test`: needs the generator; tested in `crates/beatscript-api/tests/forms_roles.rs`) | Phrase templates (roles) | `ext-roles.md` |
| `such-stuff-as-dreams-122-reference.beatscript` | the found-phrases reference program (deep house, 122): Prospero's lines from The Tempest sung buried in `night` and forward in `spot`, crossed onto the shaker and hats; the hand-written twin of `such-stuff-as-dreams-122.beatscript` | Found phrases | `found-phrases.md` |
| `tyger-tyger-128.beatscript` | the found-phrases reference program (EDM, 128): Blake's trochaic Tyger chanted on the kick (`@ vox xfer kick4`), pronunciation fixed with phoneme escapes | Found phrases | `found-phrases.md` |
| `techno-radio.beatscript` | a station, `station { }` with `from gen@2 techno seed=42`, `next seed=~` and `forms seeded passacaglia rondo fugue`: an endless program whose track N is always the same, each chapter's first seed its theme (`skip-test`: needs the generator; tested in `crates/beatscript-api/tests/station.rs`) | Radio: stations | `radio.md` |
| `showcase.beatscript` | a whole track that uses the features together (written beside these) | | |

Try one from a shell, with no app running:

```text
djctl btw check examples/beatscript/tempo.beatscript         # errors, warnings, notes, and the length of the song through its tempo
djctl btw fmt examples/beatscript/arith.beatscript           # the canonical text (what the CID is of) and the CID
djctl btw test examples/beatscript/assert.beatscript         # judge its assertions on an offline render; exit 1 when one fails
djctl btw jsonld examples/beatscript/linked.beatscript       # the program as linked data
djctl btw render examples/beatscript/mix.beatscript out.wav  # play it offline (sections, nets, tempo, duck and mix; not the other control lines)
```

`examples/trance-138.beatscript` (a 64-bar trance track) and `examples/markers-demo.beatscript` (the editor-marks screenshots) are older programs
outside this folder; `docs/beatscript/hand-dubstep-140-v2.beatscript` is the whole track of BEATSCRIPT.md's "Example: a whole track".

## deep-house-60.beatscript

One hour of hand-written deep house (no generator): 122 BPM, A minor / C major, 1824 bars = 59:48,
CC0. Program CID `z4EBG9j6BYUoMCVxaNenSpDHnPaYZ1JaBgTy6g5TpvbUVjuMpWa`. `djctl btw check` is clean.
A section is at most 256 bars, so the long movements are chained nets; every section has its own
`evolve every 16 bars` block, with coprime clocks on melody (7/11/13), voicing octaves (5/7), bass
rotate + octave (11/13/17) and hats (`thin` on `gen % 7 == k`).

| section | bars | starts at bar | at |
|---|---|---|---|
| dawn | 224 | 0 | 0:00 |
| warmth | 256 | 224 | 7:20 |
| break1 | 16 | 480 | 15:44 |
| groove | 256 | 496 | 16:15 |
| turn | 32 | 752 | 24:39 |
| depth | 256 | 784 | 25:42 |
| break2 | 16 | 1040 | 34:05 |
| lift | 256 | 1056 | 34:37 |
| afterglow | 192 | 1312 | 43:00 |
| drift | 224 | 1504 | 49:18 |
| outro | 96 | 1728 | 56:39 |

Evolve period per section (`check`): dawn 715 gens, warmth 385, groove 1547, turn 385, depth 715,
lift 238, afterglow 91, drift 715 (16 bars a generation). A section plays at most 16 generations, so
no generation repeats in the hour, and the song ends after `outro`.
