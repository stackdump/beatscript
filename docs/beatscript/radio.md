# Radio: stations

Long note for docs/BEATSCRIPT.md "Radio: stations": stations v1, phase 1. Code:
`beatscript-api::station` (`station_json`, the one answer of the MCP `station` tool, `POST /api/station` and the page's wasm
`beatscript_station`) and the `station` and `set_next` productions of `crates/djlive/grammar.ebnf`. Tests:
`crates/beatscript-api/tests/station.rs`. Example: `examples/beatscript/techno-radio.beatscript`.

## What a station is

A **station** is an endless, deterministic program of programs. Position N of a station is always the same track: the same
text, the same CID, the same notes, on any host and at any time. Nobody writes a station's tracks one by one, as a set's are
written; a walk of the seeded generator gives them, one position after another, and it never ends.

A station is itself a BeatScript program (CC0, with a CID of its own). It says three things: where the walk starts (the first
track's generator call), how it steps from one track to the next, and the forms the tracks take in turn.

Listeners do not steer a station. They **tune** it, turning one of its knobs, and they **lock** knobs. Turning a knob moves
the listener to a neighbouring station that already exists, at the same position N: every station is its text, and the
neighbour's text is there to be read before anyone turns to it. A small turn must be a small musical change (the locality
rule, below).

## The program

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

- **A station program holds the `station { }` block, `tempo`, `key` and comments, nothing else**, as a set holds its `set`
  block. A deck line, a `section`, an `arrange` or a `set` beside it is an error that says so.
- **`from gen@V GENRE seed=N [length=short|standard|long]`** is the first track's generator call ("Call the generator" in
  docs/BEATSCRIPT.md), and a station needs one. `section=`, `part=`, `bpm=` and `key=` are refused at the param: a station's tempo and
  key are its own `tempo` and `key` lines.
- **`next seed=+N`** (N at least 1) or **`next seed=~`** is the step from one track to the next (the walk, below). Default
  `next seed=+1`.
- **`forms NAME ...`** is the form rotation: track N plays `forms[N mod len]`. The names are `seeded` (the generator's own
  arrangement) and the six forms that take seeds in their roles: `passacaglia`, `canon`, `rondo`, `fugue`,
  `theme-and-variations` and `cross` (docs/beatscript/forms.md). A name may come twice (`forms seeded seeded cross`). Default
  `forms seeded`.
- **`tempo` and `key`**: every track and every segue is generated at this tempo and in this key (the calls are fitted, as any
  gen call is). A station that does not say them has the from call's own: the tempo and key of the program `gen@V GENRE
  seed=N` prints.
- **The version**: `#! stations: v1` (also what a station with no such line means). Any other value is an error. A change that
  moved any track of any station would be stations v2; v1 stays as it is.
- **The CID** of a station is the CID of its canonical text, like any program's. Comments and `#!` lines are not in it.

## The walk

The walk says which seed is at position N. s_0 is the from call's seed; every later seed comes from the step.

```text
s_0          = the from call's seed
seed=+n      s_N     = (s_0 + n*N) mod 2^32
seed=~       s_{N+1} = fold32(CID of the program "arrange from gen@V GENRE seed=s_N [length=L]")
fold32       FNV-1a 32 over the CID's bytes
```

- **A word seed** (`seed=midnight`) is FNV-1a 32 of its lowercased text, as the generator reads it. Track 0 keeps the word in
  its text.
- **`+n` counts on**: s_N is one sum, whatever N is.
- **`~` draws each seed from the track before**: s_{N+1} is "seeded from the previous track's CID", and the track the walk
  hashes is the generator call itself, `arrange from gen@V GENRE seed=s_N`, with the from call's `length=` when it has one.
  The walk is N hashes long, so a `~` station is read up to N = 10000 (some 800 hours of radio); a position past it is refused. A `+N` station has a track at any position.
- **What moves the walk.** The tempo, the key and the form of a track are not in the call the walk hashes, so turning the
  `tempo`, `key` or `forms` knob keeps every seed. Under `~` the genre and the length are in it (and the generator's version),
  and turning either gives another walk after track 0. Under `+n` only the `seed` and `step` knobs move the seeds.

## The track at N

The form of track N is F = forms[N mod R], R the length of the rotation. Each pass of the rotation is a **chapter**:
c = N div R, and the chapter's **theme** is t = s_{c*R}, the seed of its first track. Every form has a theme role, bound to t,
so the theme recurs through the chapter ("a theme bound as a role recurs across a station"); the form's other role takes the
track's own fresh seed s_N.

| form | the track's `arrange from` line | theme role (t) | fresh role (s_N) |
|---|---|---|---|
| `seeded` | `arrange from gen@V G seed=s_N` | | |
| `passacaglia` | `template(passacaglia, @PHRASE, ground=..., upper=...)` | `ground` | `upper` |
| `canon` | `template(canon, @PHRASE, subject=..., ground=...)` | `ground` | `subject` |
| `rondo` | `template(rondo, @PHRASE, refrain=..., episodes=... part=arp)` | `refrain` | `episodes`, `part=arp` |
| `fugue` | `template(fugue, @PHRASE, subject=..., counter=...)` | `subject` | `counter` |
| `theme-and-variations` | `template(theme-and-variations, @PHRASE, theme=...)` | `theme` | (none: its one role is the theme) |
| `cross` | `template(cross, @PHRASE, a=..., b=...)` | `a` | `b` |

- **Each binding is a seed of the station's call**: `gen@V G seed=t` or `gen@V G seed=s_N`, the from call's version and genre.
  The roles are written in the form's order, as every role is in the canonical text (the canon's `subject` before its
  `ground`).
- **The sparse genres** (ambient, bossa, jazz, lofi) play no arp and no harmony on the bar's grid, so there the rondo's
  `episodes` say `part=melody` and both of the cross's bindings say `part=melody`.
- **A chapter's first track binds the theme twice.** At N = c*R the fresh seed is the theme (s_N = t), so the rotation's first
  form, when it is not `seeded`, binds one seed to both roles.
- **The theme and variations is the chapter's theme varied.** Its one role takes t, so its own seed s_N picks only its phrase.
- **The phrase**: each form is sung in one section (docs/beatscript/forms.md), and a station's phrase is `@<id>`, line s_N mod 46 of a
  pinned list of 46 ids of the public-domain phrasebook. Stations v1 pins the list, so a new line in the phrasebook never moves a
  track. A `seeded` track has no phrase.

**The track's text** is a `tempo` line, a `key` line and the `arrange from` line, under header lines that say where it came from:

```text
#! station: <the station's CID> track 7
#! from: next(<the CID of track 6>) seed=~
tempo 128
key am
arrange from template(fugue, @<phrase>, subject=gen@2 techno seed=<s_4>, counter=gen@2 techno seed=<s_7>)
```

- `#! station: <cid> track N` is on every track and `#! from: next(<cid of track N-1>)`, with the station's step, on every
  track from 1 (provenance). Both are comments, not in the CID.
- **Its CID** is the CID of its canonical text: the references and all, never their expansion. Its **expanded_cid** is the CID
  of the program written out (every form and seed as plain BeatScript), which `check` gives.

## The segue

Between track N and track N+1 is a **segue**: the existing `cross` form, the two tracks' seeds crossed.

```text
tempo 128
key am
arrange from template(cross, @<phrase>, a=gen@2 techno seed=<s_N>, b=gen@2 techno seed=<s_N+1>)
```

- Its phrase is line (s_N + s_{N+1}) mod 46 of the station's list (the sum taken whole: it does not wrap at 2^32). In the sparse genres both bindings say `part=melody`.
- **A segue is 192 bars**, the length of a cross. That is a taste call of phase 1: a short segue form is phase 2.
- Each segue is a program with its own CID, and a player plays track N, the segue from N to N+1, then track N+1.

## Knobs, notches, retunes and locks

A station has seven **knobs**. A **retune** sets one; the station's position N is kept, so the listener hears track N of the new
station. One **notch** of a knob is a neighbour:

| knob | what it is | one notch |
|---|---|---|
| `genre` | the from call's genre | none: in phase 1 a genre is retuned only to a value said explicitly (its neighbourhoods are phase 2) |
| `seed` | the from call's seed, s_0 | +1 or -1 |
| `step` | the `next` line | `+1` to `~`, and `~` to `+1` |
| `forms` | the rotation | rotated by one (`seeded rondo fugue` to `rondo fugue seeded`) |
| `tempo` | the `tempo` line, 40 to 300 | +2 or -2 |
| `key` | the `key` line | one step round the circle of fifths in its mode (am to em or dm) |
| `length` | the from call's `length=` | one step along short, standard, long |

- **A lock** is a knob held fixed. The neighbours are one notch along each unlocked knob, never along a locked one, and a
  retune of a locked knob is refused.
- A retune is set **before anything else** in a request, so the tracks, segues and neighbours of the answer are the new
  station's.

### The locality rule

A small turn must be a small change, and each knob moves one dimension of the station and nothing else:

- **`tempo`**: the same walk, forms and phrases; every track generated two BPM away.
- **`key`**: the same walk, forms and phrases; every track generated in the neighbouring key.
- **`forms`**: the same walk, phrases and chapters (the themes are the same seeds); each position plays the next form of the
  rotation.
- **`length`**: under `+n`, the same seeds, each track one length longer or shorter. Under `~` the length is in the hashed call,
  so after track 0 the walk is another one.
- **`step`**: the same track 0; after it, another walk.
- **`seed`**: under `+n`, every seed one away (s_N + 1 or s_N - 1); under `~`, another walk from track 0.
- **`genre`**: a value said explicitly, the same positions in another genre.

Two adjacent seeds of the generator are different tracks: seed 42 and seed 43 are two tracks of the same genre, tempo, key and
form, and seed 43 is not a variation of seed 42. What keeps a seed neighbour close is the station's structure (the
same form at each position, the same tempo, key and length, the chapters' rhythm) and the recurring theme of each chapter. How
close they come is measured by `listen` (its loudness per section, the tempo and key it hears), not promised by the seed.

## `next` in a set

The step of a station is also a track of a set (docs/BEATSCRIPT.md "Sets"):

```text
set {
  play gen@2 techno seed=42
  mix 16 bars with crossfade
  next seed=+1
  mix 16 bars with echo_out
  next seed=~
}
```

- **`play`** takes a generator call as a track (`play gen@2 techno seed=42`), besides a saved program's name or CID.
- **`next seed=+N` or `next seed=~`** is a track: the track before it (a `play` of a generator call, or another `next`) with
  its seed stepped by the station's rule (`+N` counts on, `~` draws the seed from the CID of the track before). A `mix` may sit
  between. `next` takes an `until` as `play` does.
- `next` after a saved program (a name or a CID) is an error: there is no seed to step.
- The canonical text keeps `next seed=~` as written: it is structural, in the text and so in the set's CID. `check` reports the
  call each `next` resolved to and its CID (`set.tracks[i].call`, `.cid`), in a note at it:
  `next: gen@2 techno seed=43 -> z4EB... (seeded@2)`.

## The answer: MCP `station` and `POST /api/station`

One function, `beatscript-api::station::station_json`, answers the MCP `station` tool, `POST /api/station` and the page's wasm
`beatscript_station`: the same request gives the same answer from each.

```text
curl -s https://beatscript.app/api/station -H 'Content-Type: application/json' \
  -d '{"station": "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  next seed=~\n  forms seeded passacaglia rondo fugue\n}\n", "n": 7, "segue": true, "check": true}'
```

| field | what it asks |
|---|---|
| `station` | the station's program text |
| `n` | the position (default 0) |
| `count` | how many tracks from `n`, 1 to 16 (default 1); with `check`, 1 to 4 (each track and segue is checked, a few seconds each) |
| `segue` | the segue after each track |
| `check` | each track and segue checked: `ok`, `expanded_cid`, `bars`, `seconds`, `errors` |
| `retune` | one or more knobs set before anything else: a value (`{"tempo": 130}`), or `"+1"` / `"-1"` for one notch up or down (`{"seed": "+1"}`) |
| `locks` | the knobs held fixed (`["tempo", "key"]`) |
| `neighbours` | the one-notch neighbours, each with its station text, its CID and its track at `n` |

The answer has this shape (the numbers and CIDs here stand in for real ones):

```json
{"ok": true, "version": "stations/1",
 "station": {"text": "...", "cid": "z...", "genre": "techno", "generator": "gen@2", "seed": 42, "step": "~",
             "forms": ["seeded", "passacaglia", "rondo", "fugue"], "tempo": 128, "key": "am", "length": "standard",
             "start_cid": "z..."},
 "n": 7,
 "tracks": [{"n": 7, "seed": 3141, "theme": 2718, "form": "fugue", "phrase": "@tempest_such_stuff",
             "text": "...", "cid": "z...", "parent": "z...",
             "check": {"ok": true, "expanded_cid": "z...", "bars": 168, "seconds": 315.0, "errors": []}}],
 "segues": [{"from": 7, "to": 8, "text": "...", "cid": "z...", "check": {"ok": true, "...": "..."}}],
 "neighbours": [{"knob": "tempo", "value": 130, "text": "...", "cid": "z...", "track": {"n": 7, "cid": "z...", "...": "..."}}],
 "retuned": ["tempo"], "locks": ["key"],
 "diagnostics": [{"severity": "error", "message": "...", "line": 3, "col": 3}]}
```

- **`version`** is `stations/1`.
- **`station`**: its canonical `text` and `cid`, the knobs as they are after any retune (`genre`, `generator`, `seed`, `step`,
  `forms`, `tempo`, `key`, `length`), and `start_cid`, the CID of `arrange from <the from call>`, where the walk starts.
- **`tracks`**, one for each position from `n`: `n`, `seed` (s_N), `theme` (the chapter's t), `form`, `phrase`, the track's
  `text` and `cid`, `parent` (the CID of track n-1, absent at 0) and, with `check`, its `check`.
- **`segues`**, with `segue`: the segue after each track, `from`, `to`, `text`, `cid` and, with `check`, its `check`.
- **`neighbours`**, with `neighbours`: one notch along each unlocked knob, each with its `knob`, `value`, `text`, `cid` and its
  `track` at `n`.
- **`retuned`** names the knobs the request set, **`locks`** the knobs held.
- **`diagnostics`**: the station program's errors and warnings, each with its `severity`, `message`, `line` and `col`.
- An error answer is `{"ok": false, "error": "...", "diagnostics": [...]}`.

## The `/radio` page

beatscript.app/radio is a player for a station. It plays track N, the segue after it, track N+1, and on, each from the
station's answer for that position (the page's wasm `beatscript_station` gives the same answer as `POST /api/station`). Its knobs are
the station's knobs: a turn is a retune, so the next track is the new station's at the same position, and a lock holds a knob
still. Nothing a listener does is written into the station they were on: a tuned station is another text, which was there
before they turned to it.

While it plays the page is a player: a transport (previous, play/pause, next, stop), a readout (the title, elapsed and total
time, the section and bar, the tempo and key), a level meter and the decks' roles lit as they fire. Under it, a playlist: the
tab's "up next" queue first (MCP's `queue` tool and the playlist itself write it: add by CID, share link or example name,
remove, reorder; double-click or Enter plays a row), then the station's next tracks, N+1, N+2, ... Each row says its render
status (none, queued, rendering, ready with the Opus link, failed) and whether it plays from the render or live in the tab
("Play from": auto, the default, plays a row from its render when the render is ready; render waits for it; live never). The
statuses are asked for sparingly (once when a row shows, again only while it renders, never more than two at once or 30 a
minute, nothing while the tab is hidden), and the page starts one render only, the next row's, only while someone is listening.

**Never dead air.** What is next is resolved while the current item plays: its text fetched and compiled into a run of its own,
or its render found ready, so the change is a swap at the boundary. When it is not ready in time (a render pending or failed, a
program that does not compile, a slow network, a queue that answers late), the page plays a bridge: a small CC0 loop at the
outgoing track's tempo and key (`handoff-model.js` `BRIDGES`), hot-reloaded into the running program so it lands on the next bar
line, looping until the next item is ready, which then comes in on a bar line. The row says "bridging". A next item that fails
is passed over for the one after it while the bridge plays on; after ten minutes of bridging with nothing ready the page stops
and says why. The one intended stop is the queue's presence pause (nobody listening). A swap from one compiled run to the next
starts the engine afresh, which costs the time it takes to install the new program's nets; a gapless swap needs the engine to
land a new program at its own bar 0 on a bar line (as a scene does), which is not built yet.

**Peek.** The `{ } code` key opens the playing program's text in a drawer, without stopping it. An edit applies on the next bar
(a hot reload, as Cmd+Enter in the editor), the row says "edited" with the edit's CID, "back to station" puts the track back, and
"keep my version" puts a link to the edit in the clipboard.

## A station on any page: `<beatscript-radio>`

The player package, `@stackdump/beatscript-player` from 0.4.0 (`site/player`), has the radio as an element beside
`<beatscript-track>`, made for a page with a strict Content Security Policy, such as a Claude artifact:

```html
<script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@0.4.0/beatscript-radio.js"></script>
<beatscript-radio station="tempo 128&#10;key am&#10;station {&#10;  from gen@2 techno seed=42&#10;  next seed=~&#10;}&#10;"></beatscript-radio>
```

- **No server.** A station's tracks are deterministic from its text, so the element computes them in the page with the
  engine's own station answer (the wasm's `beatscript_station`, the function behind `POST /api/station`). Nothing is fetched
  from beatscript.app; its links there are plain links. It stores nothing: no cookies, no localStorage.
- **What it is**: the `/radio` page's player on the same models (`station-model.js` for the knobs, the answer and the play
  order, `handoff-model.js` for what is next and its CC0 bridges, `playlist-model.js` and `queue-model.js` for the rows and the
  presence pause, `driver.js`): a transport, now playing, the section and bar, the tempo and key, a level meter, the playlist
  of the station's next tracks (and the segue after the one playing) with each row's status, and the peek drawer. It has no
  queue and no renders: it plays every item live in the page.
- **Never dead air, and no gap.** The next item is compiled while the current one plays and primed on an engine of its own:
  its nets installed, its decks held. Half a beat before the current item's last bar line (or at once, on Next) the held
  decks are let go, and the outgoing engine fades out over 120 ms under the incoming one. When the next item is not ready
  in time, the bridge plays, as on `/radio`. `site/test/player-radio.mjs` measures it: across every track change, no
  silence on the element's output longer than one 1024-frame buffer.
- **Attributes**: `station` (the text, or its CID with the text in a child `<script type="text/beatscript">`), `cid` (the CID the
  text must have), the knobs `genre`, `seed`, `step`, `forms`, `tempo`, `key` and `length` (each a retune before it plays),
  `locks`, `n` (the position), `segues="off"`, `autoplay` (off by default: a Claude artifact needs a click), `compact`,
  `theme` and `site`. Its README has them in full.

The MCP `artifact` tool makes the page: `{"station": "<the station's text>"}` (or the CID of a station stored on the site,
or the name of an example that is one; `n` for the position) answers a whole HTML page with `<beatscript-radio>` from jsDelivr,
pinned to one exact version with Subresource Integrity on every module, the station's text and the CID it must have. Publish
it as it is.

## An example station: techno radio

`examples/beatscript/techno-radio.beatscript` is the station at the top of this note: techno from the seeded generator v2, seed 42,
`next seed=~`, the rotation `seeded passacaglia rondo fugue` (R = 4), at 128 in A minor. Its first tracks, form by position:

| N | chapter, theme t | form | the track's `arrange from` line |
|---|---|---|---|
| 0 | 0, 42 | `seeded` | `gen@2 techno seed=42` |
| 1 | 0, 42 | `passacaglia` | `template(passacaglia, @byron_sheen_of_their_spears, ground=gen@2 techno seed=42, upper=gen@2 techno seed=878718783)` |
| 2 | 0, 42 | `rondo` | `template(rondo, @hopkins_pied_beauty_swift_slow, refrain=gen@2 techno seed=42, episodes=gen@2 techno seed=424155996 part=arp)` |
| 3 | 0, 42 | `fugue` | `template(fugue, @percy_patrick_spens_king, subject=gen@2 techno seed=42, counter=gen@2 techno seed=3893902092)` |
| 4 | 1, 2771245628 | `seeded` | `gen@2 techno seed=2771245628` |
| 5 | 1, 2771245628 | `passacaglia` | `template(passacaglia, @lear_derry_down_derry, ground=gen@2 techno seed=2771245628, upper=gen@2 techno seed=4075880107)` |
| 6 | 1, 2771245628 | `rondo` | `template(rondo, @percy_patrick_spens_king, refrain=gen@2 techno seed=2771245628, episodes=gen@2 techno seed=2151947734 part=arp)` |
| 7 | 1, 2771245628 | `fugue` | `template(fugue, @hopkins_pied_beauty_glory, subject=gen@2 techno seed=2771245628, counter=gen@2 techno seed=4043716043)` |

Here 878718783 = fold32(CID of `arrange from gen@2 techno seed=42`), each next seed the same hash of the call before, and each
phrase line s_N mod 46 of the pinned list. Seed 42 is the ground, the refrain and the subject of the first chapter; 2771245628 is
all three in the second. The station's answer gives every number (`"count": 8`; with `"check": true` at most 4 a request).

Track 0 and track 1 in full:

```text
#! station: z4EBG9j79FsQaBchMGcKHL55KLeH6FjsjLQ7V1AE9oj2PencNUE track 0
tempo 128
key am
arrange from gen@2 techno seed=42
```

```text
#! station: z4EBG9j79FsQaBchMGcKHL55KLeH6FjsjLQ7V1AE9oj2PencNUE track 1
#! from: next(z4EBG9j8WpGPQpiVt6DgPnDa5Luuo9hDJCXk9akNi5dsyUFcRie) seed=~
tempo 128
key am
arrange from template(passacaglia, @byron_sheen_of_their_spears, ground=gen@2 techno seed=42, upper=gen@2 techno seed=878718783)
```

and the segue between them, 192 bars, its phrase line (42 + 878718783) mod 46 (CID z4EBG9j1xkcjngCue9QURULs1W3tpbaxrd3CtFWsZdffBbYkxJf):

```text
tempo 128
key am
arrange from template(cross, @hiawatha_should_you_ask, a=gen@2 techno seed=42, b=gen@2 techno seed=878718783)
```

Its neighbours, one notch along each knob: `tempo 126` and `tempo 130` (the same seeds and forms, two BPM away), `key em`
and `key dm` (the same seeds and forms in the keys either side round the circle), `seed=41` and `seed=43` (two other walks),
`next seed=+1` (the walk 42, 43, 44 ..., so its track 1 is the passacaglia of ground 42 and upper 43),
`forms passacaglia rondo fugue seeded` (track 0 a passacaglia with seed 42 in both roles), and `length=short` and
`length=long` (under `~`, two other walks). It has no `genre` notch.

## `check`

- In a station program: a station with no `from`; a `from` that says `section=`, `part=`, `bpm=` or `key=` (at the param); a
  `next seed=+0`; a form that is not one of the seven; a `#! stations:` line other than `v1`; and anything beside the
  `station` block but `tempo`, `key` and comments: each is an error at it.
- `check` of a station program compiles its track 0 and checks its first chapter and the segue from track 0 to track 1.
- In a request: a `count` outside 1 to 16, a position past 10000 on a `~` station, a retune of a locked knob, and a notch
  (`"+1"`, `"-1"`) of `genre` are refused, in the error answer.
- With `check`, each track and segue is checked as any program is (its gen calls resolved, its form written out), and its
  `check` says `ok`, `expanded_cid`, `bars`, `seconds` and `errors`.

## Not yet (phase 2)

- **Genre neighbourhoods from the ontology**: a notch of `genre`, the genres next to this one in the bitwrap ontology. In phase
  1 a genre is retuned only to a value said explicitly.
- **Vibes** and **zoom levels**: named in the design, not specified in stations v1.
- **Sessions as sets**: what a listener heard, written down as a set of the station's tracks and segues.
- **A short segue form**: a segue is a whole cross today, 192 bars.
- **A stored program as a start**: `from` takes a generator call only (a saved program has no seed to step, as with `next` in
  a set).
