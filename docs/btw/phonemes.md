# BeatScript phonemes: lyrics that read like code

*Phoneme set `beatscript-v1` (CC0). Module `djengine::vocal::phoneme`; the lexicon `crates/djengine/data/lexicon.txt`, the phrasebook
`crates/djengine/data/phrasebook.txt`, the public-domain one `phrasebook-pd.txt`; tests in `phoneme.rs`, `crates/btwapi/tests/web_vocals.rs`, `btwsite/test/wasm-vocals.mjs`.*

Write what the voice sings, exactly, between slashes:

```
deck A: sing "/w ee - aa: r - dh uh - r ay! v/" on melody
```

Text between two slashes is **phonemes**: space-separated symbols from the table below. No dictionary is asked, so it sings the
same on every engine, in every build, forever: an escape is part of the program's text, and the program's CID is its text.
Plain words still work (`sing "we are the rave" on melody`); the two mix freely (`sing "we are /dh uh/ rave" on melody`).

| in an escape | means |
|---|---|
| `aa`, `sh`, ... | one phoneme |
| ` - ` | a word break (syllables come from the vowels: one vowel, one syllable, one note) |
| `_` | a rest: the note is not sung |
| `aa:` | held: the syllable takes one more note (it lasts until the end of the next one) |
| `ay!` | accented: sung harder |
| `aa:!` | both |

**A phrase break is a `/` with a space after it** (or at the very end of the lyric). An escape opens at a `/` with a phoneme
right after it and runs to the next `/`, so a break can sit beside an escape and between plain words:

| written | read as |
|---|---|
| `"/h ii/ ! / /h ii/ !"` | an escape, a break, an escape |
| `"/ /h ii/"`, `"/h ii/ /"`, `"/h ii/!/ /h aa/"` | a break before an escape, after one, after an escape's own `!` |
| `"deep er in to night / hold me close / we move slow"` | plain words, a break, plain words, a break, plain words |
| `"/ /"`, `"/h ii/ / / /h aa/"` | two breaks in a row |
| `"/h ii"`, `"go /h ii"` | a `/` that opens an escape and has no partner is a break too, as it always was |
| `"/ w ee /"` | two breaks round the plain words `w ee`: an escape opens on a phoneme, not on a space |
| `"hello // world"` | an error at the two slashes: an escape with nothing in it (write phonemes between them, or a break as a `/` with a space after it) |

**An annotated escape keeps the word:** `word=/phonemes/` (`jingle=/j ii ng g uh l/`) sings the escape exactly as the
plain escape does, and keeps the word written beside it, so the program still says what was sung. The word is the escape's label:
it is not sung, and `check` does not report it as a word the lexicon lacks. It is letters and apostrophes touching the `=`
(`road =/r oh d/`, with a space, is the word `road` and then an escape, both sung, as before). BeatScript's tools collect these
pairs to grow the lexicon ([lexicon-harvest.md](lexicon-harvest.md)), so prefer `word=/…/` to a bare escape when the escape spells a
word.

| written | sung as |
|---|---|
| `"jingle=/j ii ng g uh l/ bells"` | `/j ii ng g uh l/ bells` |
| `"o'er=/oh r/ the hills"` | `/oh r/ the hills` |
| `"lan-tern=/l ae n t er n/"` | `lan /l ae n t er n/` (a hyphen ends the label) |

`check` reads every symbol: one it does not know is an error at its
own line and column with what it probably meant (`AH0` → `uh`, `ʃ` → `sh`, `EE` → `ee`, `shh` → `sh`). Fixes only ever add or
respell; nothing you wrote is dropped.

## The set

15 vowels and 24 consonants, readable lowercase, and `_`. Every symbol has a sound of its own and none renders silent (the
engine's test `every_consonant_alone_sounds_as_its_class` plays each consonant alone and checks its source). The IPA and ARPAbet
columns are mappings for people bringing a pronunciation from elsewhere; the symbols are BeatScript's own.

| symbol | sounds like | IPA | ARPAbet | class | how the voice makes it |
|---|---|---|---|---|---|
| `aa` | fAther | ɑ | AA | vowel | the voice through the vowel's formant targets (F1-F5) |
| `ae` | cAt | æ | AE | vowel | the voice through the vowel's formant targets (F1-F5) |
| `uh` | bUt, About | ʌ, ə | AH | vowel | the voice through the vowel's formant targets (F1-F5) |
| `aw` | lAW | ɔ | AO | vowel | the voice through the vowel's formant targets (F1-F5) |
| `eh` | bEd | ɛ | EH | vowel | the voice through the vowel's formant targets (F1-F5) |
| `er` | bIRd | ɝ | ER | vowel | the voice through the vowel's formant targets (F1-F5) |
| `ii` | bIt | ɪ | IH | vowel | the voice through the vowel's formant targets (F1-F5) |
| `ee` | sEE | i | IY | vowel | the voice through the vowel's formant targets (F1-F5) |
| `uu` | bOOk | ʊ | UH | vowel | the voice through the vowel's formant targets (F1-F5) |
| `oo` | fOOd | u | UW | vowel | the voice through the vowel's formant targets (F1-F5) |
| `ay` | dAY | eɪ | EY | diphthong | the formants glide from the first vowel's targets to the second's |
| `ie` | mY | aɪ | AY | diphthong | the formants glide from the first vowel's targets to the second's |
| `oy` | bOY | ɔɪ | OY | diphthong | the formants glide from the first vowel's targets to the second's |
| `ow` | nOW | aʊ | AW | diphthong | the formants glide from the first vowel's targets to the second's |
| `oh` | gO | oʊ | OW | diphthong | the formants glide from the first vowel's targets to the second's |
| `p` | Pop | p | P | stop | a closure, then a burst at the place's frequency (voiceless: breath after it) |
| `b` | Bass | b | B | stop | a closure, then a burst at the place's frequency (voiceless: breath after it) |
| `t` | Top | t | T | stop | a closure, then a burst at the place's frequency (voiceless: breath after it) |
| `d` | Drop | d | D | stop | a closure, then a burst at the place's frequency (voiceless: breath after it) |
| `k` | Kick | k | K | stop | a closure, then a burst at the place's frequency (voiceless: breath after it) |
| `g` | Go | ɡ | G | stop | a closure, then a burst at the place's frequency (voiceless: breath after it) |
| `f` | Feel | f | F | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `v` | Voice | v | V | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `s` | Sun | s | S | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `z` | Zoo | z | Z | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `sh` | SHine | ʃ | SH | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `zh` | meaSure | ʒ | ZH | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `th` | THin | θ | TH | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `dh` | THis | ð | DH | fricative | noise shaped at the place's frequency (voiced: over a buzz) |
| `h` | Hands | h | HH | aspirate | breath through the tract |
| `m` | Move | m | M | nasal | a dark, closed-mouth hum (the murmur) |
| `n` | Night | n | N | nasal | a dark, closed-mouth hum (the murmur) |
| `ng` | siNG | ŋ | NG | nasal | a dark, closed-mouth hum (the murmur) |
| `l` | Light | l | L | liquid | voiced; the formants glide from the consonant's place |
| `r` | Rave | ɹ | R | liquid | voiced; the formants glide from the consonant's place |
| `w` | We | w | W | glide | voiced; a fast formant glide into the vowel |
| `y` | Yeah | j | Y | glide | voiced; a fast formant glide into the vowel |
| `ch` | CHant | tʃ | CH | affricate | a burst, then frication |
| `j` | Jump | dʒ | JH | affricate | a burst, then frication |
| `_` | (silence) | | | rest | a rest: the note is not sung |

## Plain words: the BeatScript lexicon, then the rules

A word that is not escaped is looked up in the **BeatScript lexicon** (425 words, version 1: common English words that
spelling rules get wrong, the words and vocables a song leans on (hello, sing, street, ah, la, na, wow ...), and the genre's own
words: bass, rave, drop, beat, tonight, higher ...). It is written in this set, by
hand, for this project, and is CC0. A word it does not have is sounded out by **letter-to-sound rules** (written here from English
spelling conventions; nothing from espeak-ng or any GPL code), and `check` says which words those were and how the rules spelled
them:

```
warning: not in the BeatScript lexicon, sounded out by the rules: `harbour` /h aa r b ow r/
  = help: if one sounds wrong, respell it or write its phonemes between slashes
```

so you can respell it (`har-ber`), escape it with its word (`harbour=/h aa r - b er/`), or propose it for the lexicon. `tools/lexicon/propose.py` reads
programs' lyrics and lists the words the lexicon lacks, with the rules' guess, for a person to review and add by hand; it never
adds anything itself.

The public tools (MIT code, CC0 lexicon): the browser's engine, beatscript.app and the public crates use only the lexicon and the rules
above. (The desktop booth may add a larger dictionary of its own behind a private crate; no public build can link it:
`scripts/check-public-build.sh`.)

## The phrasebook

Stock calls, pre-phonemized and tuned by hand, each with its syllable count and a suggested rhythm (16 steps a bar, `x` a
syllable). Original phrases written for BeatScript; no song lyrics. Sing one by its id:

```
deck A: sing @hands_up on melody
```

A phrase is its phonemes, so it sings the same everywhere. Its rhythm is a suggestion for the part you sing it on: one note per
`x`.

| phrase | says | phonemes | syllables | rhythm |
|---|---|---|---|---|
| `@hands_up` | put your hands up | `p uu t - y aw r - h ae n d z - uh p` | 4 | `x.x.x...x.......` |
| `@feel_the_bass` | feel the bass | `f ee l - dh uh - b ay s` | 3 | `x.x.x...........` |
| `@we_are_the_rave` | we are the rave | `w ee - aa r - dh uh - r ay v` | 4 | `x.x.x...x.......` |
| `@drop_it` | drop it | `d r aa - p ii t` | 2 | `x.x.............` |
| `@one_more_round` | one more round | `w uh n - m aw r - r ow n d` | 3 | `x...x...x.......` |
| `@lights_down` | lights down | `l ie t s - d ow n` | 2 | `x...x...........` |
| `@everybody_move` | everybody move | `eh v - r ee - b aa - d ee - m oo v` | 5 | `x.x.x.x.x.......` |
| `@here_we_go` | here we go | `h ii r - w ee - g oh` | 3 | `x.x.x...........` |
| `@bring_it_back` | bring it back | `b r ii ng - ii t - b ae k` | 3 | `x.x.x...........` |
| `@jump_jump` | jump jump | `j uh m p - j uh m p` | 2 | `x...x...........` |
| `@turn_it_up` | turn it up | `t er n - ii t - uh p` | 3 | `x.x.x...........` |
| `@feel_the_beat` | feel the beat | `f ee l - dh uh - b ee t` | 3 | `x.x.x...........` |
| `@let_the_bass_drop` | let the bass drop | `l eh t - dh uh - b ay s - d r aa p` | 4 | `x.x.x........... x...............` |
| `@higher_higher` | higher higher | `h ie - er - h ie - er` | 4 | `x.x............. x.x.............` |
| `@hey_hey` | hey hey | `h ay - h ay` | 2 | `x...x...........` |
| `@oh_oh_oh` | oh oh oh | `oh - oh - oh` | 3 | `x..x..x.........` |
| `@no_sleep_tonight` | no sleep tonight | `n oh - s l ee p - t uh - n ie t` | 4 | `x...x...x.x.....` |
| `@sunrise_city` | sunrise city | `s uh n - r ie z - s ii - t ee` | 4 | `x.x.x.x.........` |
| `@everybody_jump` | everybody jump | `eh v - r ee - b aa - d ee - j uh m p` | 5 | `x.x.x.x.x.......` |
| `@louder_now` | louder now | `l ow - d er - n ow` | 3 | `x.x.x...........` |
| `@the_floor_is_yours` | the floor is yours | `dh uh - f l aw r - ii z - y aw r z` | 4 | `x.x...x.x.......` |
| `@deeper_deeper` | deeper deeper | `d ee - p er - d ee - p er` | 4 | `x.x............. x.x.............` |
| `@one_two_three_four` | one two three four | `w uh n - t oo - th r ee - f aw r` | 4 | `x...x...x...x...` |
| `@bass_in_your_chest` | bass in your chest | `b ay s - ii n - y aw r - ch eh s t` | 4 | `x...x.x.x.......` |
| `@rave_till_sunrise` | rave till sunrise | `r ay v - t ii l - s uh n - r ie z` | 4 | `x...x...x.x.....` |
| `@spin_it_back` | spin it back | `s p ii n - ii t - b ae k` | 3 | `x.x.x...........` |
| `@feel_the_energy` | feel the energy | `f ee l - dh uh - eh n - er - j ee` | 5 | `x.x.x.x.x.......` |
| `@here_comes_the_drop` | here comes the drop | `h ii r - k uh m z - dh uh - d r aa p` | 4 | `x.x.x...x.......` |

## The public-domain phrasebook

Lines of verse, scripture, speech and work song, each **first published before 1929**, to sing by id: `crates/djengine/data/phrasebook-pd.txt`,
beside the stock phrasebook, and sung the same way:

```
tempo 80
key am
deck A: melody <[a3 c4 e4 g4 a4 g4 e4 c4 a3 c4 e4 g4 a4 g4 e4 c4] [a3 c4 e4 g4 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~]>
deck A: sing @tempest_such_stuff on melody
```

The file is CC0, and every text in it is in the public domain: it was first published before 1929 and is quoted from the pre-1929 edition
its line names, never from a modern edition or translation. It is curated by hand, not scraped: 46 lines in ten families, three to five lines
each, with the year and the edition of every one checked. A line takes as many notes as it has syllables (the table says how many), so
give it a part with that many notes, or `bars=` that covers them; `check` warns, as for any lyric, when the notes run out.

What each line carries (one line of the file, fields separated by ` | `):

| field | is |
|---|---|
| `id` | what a program writes after `@` |
| `family` | one of the families below |
| `text` | the words as printed, spelling kept, `/` a line end or a pause |
| `lyric` | the same words as BeatScript sings them: a word of the BeatScript lexicon as it is, every other word as a phoneme escape (`/s uh ch - s t uh f/`), ` / ` a phrase break. Where the scansion needs a different count than the dictionary (`Assyrian` in three, `Heav'n` in one) the escape carries the reading. `sing "<lyric>" on melody` checks with no warning |
| `work`, `author`, `year` | where it is from, and when it was first published |
| `edition`, `source` | the pre-1929 edition the text was read from, and where it can be read online |
| `meter` | the meter as a person names it |
| `feel` | how it sits on the grid: `straight` or `triplet` (below) |
| `stress` | the meter as a step pattern, one step a syllable: `x` stressed, `~` not (`~ x` is an iamb), written as `deck A: hihat ~ x ~ x` reads it |
| `syllables` | how many notes it takes: the steps of `stress`, the syllables of `lyric` |
| `call` | for call and response, how many of the first syllables are the leader's call (the rest is the answer); 0 otherwise |
| `note` | one sentence on how it scans, or on a caveat in the date or the text |

Meter is musical structure: a stressed syllable is an accent step, a foot is a subdivision (iambs and trochees sit on straight sixteenths,
dactyls and anapests go in threes), and the length of a line is the length of a phrase. The file records all of it; `sing` itself leaves the
rhythm to the part it is sung on.

| family | what a line of it is |
|---|---|
| `iambic` | Lines of ten syllables (eleven with a feminine ending) in pairs, unstressed then stressed: the beat of blank verse and the sonnet. |
| `trochaic` | A stressed syllable then an unstressed one, four to a line, the last often cut short: Blake's Tyger and Hiawatha. |
| `anapestic` | Two unstressed syllables then a stressed one, so it rolls in threes: Byron's Sennacherib and Lear's nonsense. |
| `dactylic` | A stressed syllable then two unstressed ones, also in threes: the long line of Evangeline. |
| `common-meter` | The hymn and ballad stanza: a line of four beats, then a line of three (eight syllables, then six). |
| `sprung` | Gerard Manley Hopkins's rhythm: count the stresses, not the syllables; a stress stands alone or takes up to three slack syllables. |
| `nursery` | Two or three strong beats a line with any number of weak syllables between: a rhyme said aloud. |
| `parallelism` | The verse of the King James Bible: two halves that answer each other (day and night, speech and knowledge), with no fixed foot. |
| `oratory` | Speech: no fixed foot, the stresses of the sentence, built in threes and answering clauses. |
| `call-and-response` | A work song: the leader sings a line and the crew answers; the stresses are the heaves, the pulls on the rope. |

| feel | means |
|---|---|
| `straight` | the syllables sit on the sixteenth grid |
| `triplet` | the syllables go in groups of three |

| phrase | says | from | year | meter | stress | syllables |
|---|---|---|---|---|---|---|
| `@tempest_such_stuff` | we are such stuff as dreams are made on / and our little life is rounded with a sleep | William Shakespeare, The Tempest, 4.1.156-158 | 1623 | iambic pentameter | `~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ x` | 20 |
| `@sonnet18_summers_day` | Shall I compare thee to a summer's day? | William Shakespeare, Sonnet 18 | 1609 | iambic pentameter | `~ x ~ x ~ x ~ x ~ x` | 10 |
| `@macbeth_tomorrow` | To-morrow, and to-morrow, and to-morrow, | William Shakespeare, Macbeth, 5.5.19 | 1623 | iambic pentameter | `~ x ~ x ~ x ~ x ~ x ~` | 11 |
| `@paradise_lost_better_to_reign` | Better to reign in Hell, then serve in Heav'n. | John Milton, Paradise Lost, Book 1, l. 263 | 1667 | iambic pentameter | `x ~ ~ x ~ x ~ x ~ x` | 10 |
| `@paradise_lost_justify_the_ways` | And justifie the wayes of God to men. | John Milton, Paradise Lost, Book 1, l. 26 | 1667 | iambic pentameter | `~ x ~ x ~ x ~ x ~ x` | 10 |
| `@tyger_burning_bright` | Tyger! Tyger! burning bright / In the forests of the night, | William Blake, The Tyger (Songs of Experience), stanza 1, ll. 1-2 | 1794 | trochaic tetrameter, catalectic | `x ~ x ~ x ~ x x ~ x ~ x ~ x` | 14 |
| `@tyger_anvil_grasp` | What the anvil? what dread grasp / Dare its deadly terrors clasp? | William Blake, The Tyger (Songs of Experience), stanza 4, ll. 15-16 | 1794 | trochaic tetrameter, catalectic | `x ~ x ~ x ~ x x ~ x ~ x ~ x` | 14 |
| `@hiawatha_should_you_ask` | Should you ask me, whence these stories? / Whence these legends and traditions, | Henry Wadsworth Longfellow, The Song of Hiawatha, Introduction, ll. 1-2 | 1855 | trochaic tetrameter | `x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~` | 16 |
| `@hiawatha_gitche_gumee` | By the shores of Gitche Gumee, / By the shining Big-Sea-Water, | Henry Wadsworth Longfellow, The Song of Hiawatha, III 'Hiawatha's Childhood' | 1855 | trochaic tetrameter | `x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~` | 16 |
| `@hiawatha_dark_behind` | Dark behind it rose the forest, / Rose the black and gloomy pine-trees, | Henry Wadsworth Longfellow, The Song of Hiawatha, III 'Hiawatha's Childhood' | 1855 | trochaic tetrameter | `x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~` | 16 |
| `@byron_assyrian_wolf` | The Assyrian came down like the wolf on the fold, | George Gordon, Lord Byron, The Destruction of Semnacherib (now spelled Sennacherib), in Hebrew Melodies | 1815 | anapestic tetrameter | `~ ~ x ~ ~ x ~ ~ x ~ ~ x` | 12 |
| `@byron_sheen_of_their_spears` | And the sheen of their spears was like stars on the sea, | George Gordon, Lord Byron, The Destruction of Semnacherib (now spelled Sennacherib), in Hebrew Melodies | 1815 | anapestic tetrameter | `~ ~ x ~ ~ x ~ ~ x ~ ~ x` | 12 |
| `@byron_leaves_of_the_forest` | Like the leaves of the forest when Summer is green, | George Gordon, Lord Byron, The Destruction of Semnacherib (now spelled Sennacherib), in Hebrew Melodies | 1815 | anapestic tetrameter | `~ ~ x ~ ~ x ~ ~ x ~ ~ x` | 12 |
| `@lear_derry_down_derry` | and with laughter they shook / At the fun of that Derry down Derry. | Edward Lear, A Book of Nonsense, title-page verse ('There was an Old Derry down Derry') | 1861 | anapestic (limerick lines) | `~ ~ x ~ ~ x ~ ~ x ~ ~ x ~ ~ x ~` | 16 |
| `@evangeline_forest_primeval` | This is the forest primeval. / The murmuring pines and the hemlocks, | Henry Wadsworth Longfellow, Evangeline, Part the First, opening | 1847 | dactylic hexameter | `x ~ ~ x ~ ~ x ~ ~ x ~ ~ x ~ ~ x ~` | 17 |
| `@evangeline_ye_who_believe` | Ye who believe in affection that hopes, / and endures, and is patient, | Henry Wadsworth Longfellow, Evangeline, Part the First, prelude | 1847 | dactylic hexameter | `x ~ ~ x ~ ~ x ~ ~ x ~ ~ x ~ ~ x ~` | 17 |
| `@evangeline_silently_one_by_one` | Silently one by one, / in the infinite meadows of heaven, | Henry Wadsworth Longfellow, Evangeline, Part the First, III | 1847 | dactylic hexameter | `x ~ ~ x ~ x ~ ~ x ~ ~ x ~ ~ x ~` | 16 |
| `@evangeline_talk_not_of_wasted` | Talk not of wasted affection, / affection never was wasted; | Henry Wadsworth Longfellow, Evangeline, Part the Second (Father Felician to Evangeline) | 1847 | dactylic hexameter | `x ~ ~ x ~ ~ x ~ ~ x ~ x ~ ~ x ~` | 16 |
| `@dickinson_chariot_death` | Because I could not stop for Death, / He kindly stopped for me; | Emily Dickinson, The Chariot ('Because I could not stop for Death') | 1890 | common meter | `~ x ~ x ~ x ~ x ~ x ~ x ~ x` | 14 |
| `@dickinson_frigate_book` | There is no frigate like a book / To take us lands away, | Emily Dickinson, 'There is no frigate like a book' (printed in 1896 as 'A Book') | 1894 | common meter | `~ ~ x x ~ ~ ~ x ~ x ~ x ~ x` | 14 |
| `@newton_amazing_grace` | Amazing grace! (how sweet the sound) / That sav'd a wretch like me! | John Newton, Amazing grace! (Hymn 41, 'Faith's review and expectation'), in Olney Hymns, Book 1 | 1779 | common meter | `~ x ~ x ~ x ~ x ~ x ~ x ~ x` | 14 |
| `@watts_god_our_help` | Our God, our help in ages past, / Our hope for years to come, | Isaac Watts, Psalm 90 in Common Metre ('Man Frail and God Eternal'), later 'O God, our help in ages past' | 1719 | common meter | `~ x ~ x ~ x ~ x ~ x ~ x ~ x` | 14 |
| `@percy_patrick_spens_king` | The king sits in Dumferling-toune, / Drinking the blude-reid wine: | Anonymous, Sir Patrick Spence, A Scottish Ballad (Child 58) | 1765 | ballad meter (common meter, alternating 4 and 3 stresses) | `~ x x ~ ~ x ~ x x ~ ~ x ~ x` | 14 |
| `@hopkins_pied_beauty_glory` | Glory be to God for dappled things— | Gerard Manley Hopkins, Pied Beauty (no. 13) | 1918 | sprung rhythm | `x ~ ~ ~ x ~ x ~ x` | 9 |
| `@hopkins_pied_beauty_swift_slow` | With swift, slow; sweet, sour; adazzle, dim; | Gerard Manley Hopkins, Pied Beauty (no. 13) | 1918 | sprung rhythm | `~ x x x x ~ x ~ x` | 9 |
| `@hopkins_pied_beauty_landscape` | Landscape plotted and pieced—fold, fallow, and plough; | Gerard Manley Hopkins, Pied Beauty (no. 13) | 1918 | sprung rhythm | `x ~ x ~ ~ x x x ~ ~ x` | 11 |
| `@hopkins_windhover_brute_beauty` | Brute beauty and valour and act, oh, air, pride, plume, here / Buckle! | Gerard Manley Hopkins, The Windhover: To Christ our Lord (no. 12) | 1918 | sprung rhythm | `x x ~ ~ x ~ ~ x ~ x x x x x ~` | 15 |
| `@hopkins_spring_and_fall_margaret` | Margaret, are you grieving / Over Goldengrove unleaving? | Gerard Manley Hopkins, Spring and Fall: to a young child (no. 31) | 1918 | sprung rhythm | `x ~ x ~ ~ x ~ ~ ~ x ~ x ~ x ~` | 15 |
| `@mgm_high_diddle_diddle` | High diddle, diddle, / The Cat and the Fiddle, | Anonymous, 'High diddle, diddle' (now 'Hey diddle diddle') | 1780 | nursery, dactylic/anapaestic dimeter | `x ~ ~ x ~ ~ x ~ ~ x ~` | 11 |
| `@mgm_hush_a_by_baby` | Hush a by Baby / On the Tree Top, | Anonymous, 'Hush a by Baby' (now 'Rock-a-bye baby') | 1780 | nursery lullaby, dactylic dimeter | `x ~ ~ x ~ ~ ~ x ~` | 9 |
| `@mgm_little_jack_horner` | Little Jack Horner / Sat in a Corner, / Eating of Christmas Pye; | Anonymous, 'Little Jack Horner' | 1780 | nursery, dactylic dimeter | `x ~ ~ x ~ x ~ ~ x ~ x ~ ~ x ~ x` | 16 |
| `@mgm_pease_porridge_hot` | Pease Porridge hot, / Pease Porridge cold, | Anonymous, 'Pease Porridge hot' (riddle: 'Spell me that in four Letters? I will. T-H-A-T.') | 1780 | nursery, accentual (clapping rhyme) | `x x ~ x x x ~ x` | 8 |
| `@psalm42_deep_calleth` | Deep calleth unto deep at the noise of thy waterspouts | The translators of the King James Version, King James Bible, Psalm 42:7 (first half, cut at the colon) | 1611 | free (parallelism) | `x x ~ ~ ~ x ~ ~ x ~ ~ x ~ x` | 14 |
| `@psalm42_waves_billows` | all thy waves and thy billows are gone over me | The translators of the King James Version, King James Bible, Psalm 42:7 (second half) | 1611 | free (parallelism) | `~ ~ x ~ ~ x ~ ~ x ~ ~ x` | 12 |
| `@eccl3_season` | To every thing there is a season | The translators of the King James Version, King James Bible, Ecclesiastes 3:1 (first clause) | 1611 | free (parallelism) | `~ x ~ x ~ ~ ~ x ~` | 9 |
| `@psalm19_day_unto_day` | Day unto day uttereth speech / and night unto night sheweth knowledge | The translators of the King James Version, King James Bible, Psalm 19:2 | 1611 | free (parallelism) | `x ~ ~ x x ~ ~ x ~ x ~ ~ x x ~ x ~` | 17 |
| `@psalm121_lift_up_mine_eyes` | I will lift up mine eyes unto the hills / from whence cometh my help | The translators of the King James Version, King James Bible, Psalm 121:1 | 1611 | free (parallelism) | `~ ~ x ~ ~ x ~ ~ ~ x ~ x x ~ ~ x` | 16 |
| `@gettysburg_four_score` | Four score and seven years ago | Abraham Lincoln, Gettysburg Address (Bliss copy), delivered 19 November 1863 | 1863 | free (oratorical prose) | `x ~ ~ x ~ x ~ x` | 8 |
| `@gettysburg_highly_resolve` | that we here highly resolve / that these dead shall not have died in vain | Abraham Lincoln, Gettysburg Address (Bliss copy), delivered 19 November 1863 | 1863 | free (oratorical prose) | `~ ~ x x ~ ~ x ~ x x ~ x ~ x ~ x` | 16 |
| `@gettysburg_new_birth` | that this nation, under God, / shall have a new birth of freedom | Abraham Lincoln, Gettysburg Address (Bliss copy), delivered 19 November 1863 | 1863 | free (oratorical prose) | `~ ~ x ~ x ~ x ~ ~ ~ x x ~ x ~` | 15 |
| `@gettysburg_of_the_people` | government of the people, by the people, for the people | Abraham Lincoln, Gettysburg Address (Bliss copy), delivered 19 November 1863 | 1863 | free (oratorical prose) | `x ~ ~ ~ ~ x ~ x ~ x ~ x ~ x ~` | 15 |
| `@shanty_blow_the_man_down` | Oh blow the man down, bullies, blow him away / Way ay! Blow the man down | Anonymous (traditional), Blow the Man Down (No. 32), halliard shanty | 1914 | work-song call and response (halliards / hauling) | `~ x ~ ~ x ~ ~ x ~ ~ x x ~ x ~ ~ x` | 17 |
| `@shanty_drunken_sailor` | What shall we do with a drunken sailor? / Way-ay and up she rises | Anonymous (traditional), Drunken Sailor (No. 7, 'Capstan Chanteys'), sung by Mr. James Tucker at Bristol | 1914 | work-song call and response (capstan / marching walk-away) | `x ~ ~ x ~ ~ x ~ x ~ x ~ ~ x ~ x ~` | 17 |
| `@shanty_leave_her_johnny` | Leave her Johnny and we'll work no more / Leave her Johnny, leave her! | Anonymous (traditional), Leave Her Johnny (No. 9) | 1914 | work-song call and response (pumping / hauling) | `x ~ x ~ ~ ~ x ~ x x ~ x ~ x ~` | 15 |
| `@shanty_sally_brown` | Seven long years I courted Sally / Way! heigh! Roll and go | Anonymous (traditional), Sally Brown ('another capstan chanty known also as Sally Brown') | 1888 | work-song call and response (capstan) | `x ~ x x ~ x ~ x ~ x x x ~ x` | 14 |
| `@shanty_haul_away_joe` | King Louis was the King o' France before the Revolution / Away, haul away, haul away, Jo | Anonymous (traditional), Haul Away, Jo. ('Song used as a last short pull for sweating up') | 1910 | work-song call and response (halliards / sweating up) | `x x ~ ~ ~ x ~ x ~ x ~ x ~ x ~ ~ x x ~ x x ~ x x` | 24 |

Dates, as honestly as they are known: `year` is the first publication of the work, and for a shanty the year of the earliest collection
in which this wording was read (a song's own first year is not knowable). The Mother Goose rhymes are dated 1780, when the collection was
entered for copyright; no copy of that printing is known, and the text is read from a reprint of about 1785 to 1787. `@lear_derry_down_derry` is dated 1861, the enlarged edition whose wording is quoted: the verse is on the 1846 title page too, but its wording there
was not seen and may differ. `@dickinson_frigate_book` is dated by the first print found (the 1894 letters). Each line's `note` says so again beside it.

The corpus is one text and has one CID: the CID of the file's text, exactly as shipped, by the function a program's CID is (so there is
no second hash scheme). beatscript.app serves it at `/phrasebook` (this table, with a sing example for each line), `/phrasebook.txt` (the
file), `/phrasebook.jsonld` (linked data, one node a line, in the vocabulary at `/schema`) and `/phrasebook/<CID>.txt` and
`/phrasebook/<CID>.jsonld`, which answer only for the CID the server ships and are `404` for any other. A new version of the corpus is a new
CID; a program that sings `@id` names the line, not the corpus, so it keeps singing whatever the corpus's version.

## In the browser

beatscript.app sings a `sing` line that starts with its section (or the program) as a part of its deck's own net: a formant-voice
part with each note's syllable (and its glide and closing consonant), the cluster consonants (the `s` and `t` of "street") on a
part beside it. So it is heard through the deck's level, EQ and the crossfader like any part, loops with the deck, and stops with
it: when the program stops or ends, when an edit takes the line out, and where a looped song comes round. A `sing` line that is
timed (`at bar 9: deck A: sing ...`) or performed (`when held(key)`) is not sung in the browser, and `check` says so there.

The vocoder plays in the browser too, with the lines the language already has:

```
do vocoder set carrier=A modulator=B insert=true on=true mix=0 -> 1 over 8 bars
before drop 1 bars: move vocoder_handoff on A
```

`carrier` and `modulator` are decks (a deck with a `sing` line on it is a vocal modulator); `insert=true` puts the vocoded sound on
the carrier's strip, so its fader and the crossfader take it; `vocoder_handoff` swaps carrier and modulator on the next phrase line.

## Grammar

```
sing     = "sing" ( STRING | PHRASE ) "on" role { "bars=" INT | "key=" KEY | "voice=" NAME } ;
PHRASE   = "@" ( "a".."z" | "_" ) { "a".."z" | "0".."9" | "_" } ;      (* a phrasebook id *)
(* inside the STRING of a sing line or of `do vocal create lyrics=` (lyric_text in crates/djlive/grammar.ebnf): *)
lyric_text = { TEXT | "/" SPACE | escape } [ "/" { TEXT } ] ;              (* TEXT: any character but "/"; "/" and a space: a phrase break *)
                                                                           (* word=/…/ (jingle=/j ii ng g uh l/): the word, TEXT touching "=", is the escape's label: kept, not sung *)
escape     = "/" PHONES "/" ;                                              (* a "/" and a phoneme: the escape runs to the next "/" *)
PHONES     = phone { ( SPACE | "-" ) phone } ;                             (* from a non-space to the next "/"; "_" is a rest too *)
phone      = SYMBOL [ ":" ] [ "!" ] ;                                      (* SYMBOL: the set above; ":" and "!" go on a vowel, either order *)
```
