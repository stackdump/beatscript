# djapp's control socket and MCP server

djapp listens on a local Unix socket and `djctl app-mcp` serves that socket as an MCP server.
Together they cover what the window and the controllers can do: audio devices and input routing,
decks, transport and tempo, the mixer, FX, loops and captures, input history, the MIDI clock and
the TD-3's clock source, TD-3 / PD705 MIDI, bindings, the HEOS zones, and app status.

```
 MCP client ──stdio──▶ djctl app-mcp ──/tmp/djapp.sock──▶ djapp (window thread)
 (val palette, over ssh)   one request per tool call        same Engine calls as the UI
```

## The socket

- **Path:** `--control PATH` (default `$DJAPP_SOCKET`, else `/tmp/djapp.sock`). `--control off`
  turns it off. On macOS the path must be under 104 bytes.
- **It's on by default and works alongside the hub flags.** With the booth's settings in the
  config file (below), the booth line is just `~/bin/djapp-booth/djapp`; flags still work and
  override the file.
  To run a second djapp next to the booth, give it another path (`--control /tmp/djapp-2.sock`).
  If something already answers on the path, djapp leaves it alone and starts without a socket
  (the status bar and stderr say so). It never takes the booth's socket over.
- **Framing:** one JSON object per line, as `djhost::control_socket` frames it (it's the Swift
  app's framing):
  `{"id":1,"method":"deck","params":{"deck":"A","action":"play"}}` →
  `{"id":1,"result":{…}}` or `{"id":1,"error":"…"}`. The socket is 0600. A line may also be a
  JSON **array** of requests: it is one batch, answered as an array (see "Batches").
- **Threads:** the socket threads never touch the engine. Each request goes to the window's
  thread, which answers it on its next pass (about every 16 ms). So a request runs through the
  same `Engine` calls as a button or a controller, and the window and the controllers' LEDs
  follow. `ping` is the one method answered on the socket thread.

From a shell: `djctl app <method> [json | key=value …] [--socket PATH]`, e.g.
`djctl app deck deck=B action=load genre=techno seed=7`, or `djctl app state section=inputs`.

## The language

One shape for everything: a **noun** (the `method`: a panel or a piece of gear), a **verb** (`action`)
and params. The registry (`djhost::control_registry`) is the single table behind `describe`, the
MCP tool list and the glance table below, so none of them can drift from what djapp answers.

```json
{"id":1,"method":"loopsets","params":{"action":"load_pair","spec":"dnb:2602","group":2}}
{"id":2,"method":"loopsets","action":"pick","spec":"dnb"}      // params may sit beside method
{"id":3,"method":"loopsets.groups"}                             // noun.verb in the method
{"id":4,"method":"describe","params":{"noun":"loopsets"}}       // every verb and param
{"id":5,"method":"state","params":{"section":"loopsets"}}       // a read
```

- **Reads** are `state section=<noun>`, or the noun's own `status` / `list` / `get`.
- **Writes** answer the noun's state after the change, so a caller reads what its call did.
- **Errors** are `<noun>: <what>` and name the choices (`unknown action "x"; one of …`).
- **`describe`** answers on the socket's thread (even while the window is busy): no params, the
  language and every noun; `noun`, its doc, verbs and params; `noun` + `action`, one verb;
  `search`, nouns and verbs by word. Each verb has an `effect`: `read`, `write`, or `live` (reaches
  the speakers, gear, network or disk: ask first unless asked).
- **Old callers keep working.** Every method and verb of before is unchanged; `djctl app <noun>
  <verb> k=v …` and `djctl app <noun>.<verb>` are only shorter ways to write `action=`.

### Methods at a glance

<!-- BEGIN GENERATED: methods at a glance (control_registry) -->
| method | MCP tool | what | actions |
|---|---|---|---|
| `status` | `djapp_status` | app, output, inputs, gear, controllers, clock, counters, notes | (params only) |
| `state` | `djapp_state` | the live state: decks, mixer, transport, loops, inputs, vocoder, tricks (`section` picks one) | (params only) |
| `audio` | `djapp_audio` | audio devices and routing: list, switch the output, re-route an input strip | **devices** · output · input |
| `deck` | `djapp_deck` | one deck: transport, load, tempo, sync, sections, cues, loops, macros, sound, copy | **state** · play · pause · toggle · stop · cue · load · demo · file · next · seed_step · tempo · pitch · pitch_range · nudge · sync · section · hot_cue · clear_cue · loop · jump · seek · scratch · role · macro · master_macro · panic · rewind · focus · sound · auto_gain · role_disable · copy |
| `perform` | `djapp_perform` | any DeckAction (the session JSON) through Engine::perform | (params only) |
| `transport` | `djapp_transport` | the transport and master tempo: start / stop / tap / lock / free tempo / auto | **state** · start · stop · toggle · tap · lock · unlock · free_bpm · auto |
| `mixer` | `djapp_mixer` | crossfader, master, headphones, a strip's volume / gain / EQ / PFL / mute / aux send | (params only) |
| `fx` | `djapp_fx` | the FX unit on a deck: set, kill, reset, params | set · kill · reset · params |
| `aux` | `djapp_aux` | the aux send / return bus and the macros over it | (params only) |
| `input` | `djapp_input` | an input strip: trim, volume, mute, PFL, EQ, channel, latency (or measure it), crossfader side | status · calibrate_latency · latency |
| `loops` | `djapp_loops` | the loop row and the capture library | **list** · removed · restore_removed · delete_removed · root · to_deck · library_to_deck · play · stop · stop_all · remove · record · grab · volume · mute · library · library_load · library_save · library_delete · library_row |
| `keys` | `djapp_keys` | where the controllers' piano keys go | **status** · mode · deck · quant · scale_lock · note · off |
| `pads` | `djapp_pads` | the MPK's pads: mode, learn, notes | **status** · learn · cancel · clear · set · feed |
| `vox` | `djapp_vox` | the vocal pads: folder, page, choke, stretch, play | **status** · dir · scan · page · set · play |
| `capture` | `djapp_capture` | capture to loop from an input's history | status · **grab** · start · end · arm · bars |
| `history` | `djapp_history` | any strip's history: scrub, live, loop, window, jog aim | **status** · touch · scrub · release · live · loop · loop_step · beats · jog · snap · grid · align · loop_bars |
| `clock` | `djapp_clock` | the MIDI clock master, PD705 steps, the TD-3's clock source | (params only) |
| `td3` | `djapp_td3` | the TD-3: keys, pattern editor, audition, write (dry run unless confirmed) | **status** · keys · octave · channel · generate · slot · step · audition · write · cutoff_cc |
| `kits` | `djapp_kits` | the PD705 kit library: kits, pads, patterns, build, install | **status** · list · get · add · remove · rename · duplicate · select · host · pad · pattern · build · install · loaded · mark_loaded · forget · track · audition · open |
| `midi` | `djapp_midi` | gear ports, the monitor, send a message, a midiOut | ports · monitor · send · out |
| `bindings` | `djapp_bindings` | devices, maps, presets, bind / unbind, press a control, the target registry | **devices** · list · map · preset · preset_add · preset_delete · bind · unbind · press · targets · inject · jog · view · add_device · import · keys · activity |
| `target` | `djapp_target` | fire any registry target, as a bound control would | (params only) |
| `route` | `djapp_route` | loop routing: a loop's parts out to gear as notes while it plays | **status** · set · toggle · clear · notes · offset · add_dest · open |
| `log` | `djapp_log` | the action log and its beat-stamped arrangement: save / load / play a passage | **lines** · arrangement · save · load · play · stop · clear · open |
| `screenshot` | `djapp_screenshot` | a PNG of the window | (params only) |
| `clips` | `djapp_clips` | the loop row in full: scenes, crops, sequences, retune, clip FX, per-clip launch settings | **list** · scene_save · scene_launch · scene_rename · scene_delete · crop · sequence · retune · fx · fx_link · key_lock · row_key_lock · quantize · row_quantize · fade · row_fade · follow · row_follow · gain · rename · color · restore · forget |
| `wander` | `djapp_wander` | the walk on each deck: status, on / off, pace, lock, grab | **status** · on · off · pace · lock · grab |
| `automix` | `djapp_automix` | auto-mix: status, set, off, now, defer | **status** · set · off · now · defer |
| `autonext` | `djapp_autonext` | auto-next every N bars on a deck | **status** · on · off · toggle · set · up · down |
| `trick` | `djapp_trick` | performance tricks on a deck: on / off / toggle a kind | **status** · on · off · toggle · knob |
| `moves` | `djapp_moves` | the Perform window's DJ moves on the phrase grid | **status** · arm · cancel · toggle · cancel_all |
| `autodj` | `djapp_autodj` | Auto-DJ: move on by itself every N bars, with effects across the handover | **status** · on · off · toggle · set · next · cancel |
| `setlist` | `djapp_setlist` | set lists: build a whole set in advance (segments of a pair / net seed / saved track, each with a transition, inserts and a tag), generate one from a recipe, save, load, edit | **list** · get · new · generate · save · load · delete · duplicate · edit · license · visibility · fork · share |
| `track` | `djapp_track` | saved tracks with a license (CC0-1.0 default | CC-BY-4.0 | all-rights-reserved) and a visibility (public | private): save a deck, relicense, fork, share by CID | **list** · get · save · license · visibility · fork · share · load · delete |
| `queue` | `djapp_queue` | the set queue: play sets back to back on the phrase grid, with the transitions and inserts they hold | **status** · add · clear · play · pause · stop · next · jump · loop |
| `btw` | `djapp_btw` | BeatScript, the bitwrap language: a text program whose pattern lines are nets on the decks and whose lines script everything djapp does; eval lands on the next bar | eval · check · **get** · save · load · list · stop · fmt · share · open · fork · license · visibility · export · import · generation · instruments · jsonld · terms · test · perform · replay |
| `freeze` | `djapp_freeze` | granular freeze on a strip's history | **status** · set · on · off · toggle · scrub · phrase |
| `vocoder` | `djapp_vocoder` | the channel vocoders: sources, knobs, switches, synth, presets, instances | **status** · set · cycle_modulator · cycle_carrier · swap · preset_save · preset_delete · preset_next · preset_prev · list · add · remove · select |
| `voice` | `djapp_voice` | the formant voice: robot / choir vocals sung by the nets (presets, vowels, a sung part, the vocoder's voice) | **status** · presets · vowel · load · vocoder |
| `vocal` | `djapp_vocal` | vocals from typed lyrics: words sung on pitch by the formant voice, CC0 by default (a line you can save, fork, license and send to the loop row, a deck or the vocoder) | **status** · words · create · get · list · save · load · license · visibility · fork · share · delete · play · to_row · to_deck · to_vocoder · hook · render |
| `render` | `djapp_render` | offline render of an arrangement or the action log (runs in the MCP server, not the socket) | (params only) |
| `post` | `djapp_post` | the post chain: live on the master, and offline on files | **status** · set · process · measure · presets |
| `heos` | `djapp_heos` | the Denon HEOS zones: volume, mute, transport, groups, the master streamed to them | **status** · volume · mute · play · pause · stop · group · ungroup · stream · stream_stop · stream_server_stop · select · discover · connect · refresh · open |
| `config` | `djapp_config` | the config file: get / set / diff / save / reload / keys / path | **get** · set · diff · save · reload · keys · path |
| `perf` | `djapp_perf` | the perf overlay and its numbers | **status** · on · off · toggle · reset · set |
| `browse` | `djapp_browse` | the loop / sample browser: search, load, audition | **status** · sources · packs · list · search · load · audition · stop · gain · open · rescan · select |
| `ping` | — | is it alive (answered on the socket's thread) | (params only) |
| `methods` | — | the method names (the nouns); describe has the verbs | (params only) |
| `describe` | `djapp_describe` | the control language itself: every noun, verb and param, from the registry | (params only) |
| `net` | `djapp_net` | djnet, the LAN protocol: pairing, peers and their clocks, tracks by CID, the phone's stream | **status** · track · glance · stream · stream_stop · stream_status · aux_sync · aux_lock · aux_nudge · aux_trim · aux_latency · aux_match · code · trust · forget · open |
| `loopsets` | `djapp_loopsets` | the Loops window: loop collections (Acid Bass, a genre's set, genre:seed), their A / B groups on the decks | **status** · collections · groups · pick · new_set · keep · drums · load · load_pair · open |
| `windows` | `djapp_windows` | the windows: which are open, and open / close / toggle any by name (the top bar's buttons) | **list** · open · close · toggle · fullscreen · follow · closeall |
| `inserts` | `djapp_inserts` | the Inserts window: filter / crush / delay / reverb on A, B, the Wheel or the Master | **status** · on · set · off |
| `effects` | `djapp_effects` | the Effects window: every trick and macro pad on A, B, the Wheel or the Master | **status** · catalogue · tap · hold · release · fire · length · sweep · knob · panic · target |
| `wheel` | `djapp_wheel` | the Wheel window: windwheel's tonewheel instrument (play, instrument, motor, key, level) | **status** · presets · play · pause · toggle · load · motor · interval · sync · step · follow · pluck · level · trigger · key · brightness · sequence |
| `controllers` | `djapp_controllers` | controllers and their Mixxx mappings: what drives each port, load / unload a mapping, what it left unmapped | **status** · list · load · unload · report · rescan · map |
| `menu` | `djapp_menu` | the ≡ Menu window: keep the computer awake, the Toy Box sound pack, About | **status** · keep_awake · toybox |
| `batch` | `djapp_batch` | several commands as one request: all or nothing, or landing together on the bar grid | **status** · run · cancel |
| `key` | `djapp_key` | musical keys: each deck's key in Camelot, how the decks mix, key detection, key lock | **status** · detect · set · lock · tempo · compat |
| `analysis` | `djapp_analysis` | beat, tempo and bar-1 detection for audio djrust did not make (Beat This! or the DSP detector) | **status** · beats · grid · set · models · fetch_models · retry |
| `mashup` | `djapp_mashup` | live mashups from stems: drums from one track, bass from another, beat-synced and key-aware, content-addressed with parent CIDs | **status** · sources · new · add · remove · stems · assign · part · key · tempo · play · save · load · list · fork · delete · open |
| `record` | `djapp_record` | set recording: the master to a WAV plus a timestamped CID tracklist and an exact replay, signed and content-addressed | **status** · start · stop · cancel · mark · list · get · tracklist · program · verify · replay · fork · delete |
<!-- END GENERATED -->

### Method notes

| method | actions / params |
|---|---|
| `ping` | is it alive (pid) |
| `methods` | the method names; `describe` has the verbs and params |
| `describe` | the registry: `noun`, `action`, `search` (above) |
| `loopsets` | the Loops window (docs/ANDROID-PARITY.md): `status` (default: the collection picked, key, tempo, CID, `held` per deck, the lit `pair`), `collections` (the picker: Acid Bass, each genre's curated set, the ★ kept sets), `groups` (`spec`?: each group's A and B loop), `pick` (`spec`: `acid-bass`, a genre, `genre:seed`; built at once), `new_set` (`genre`?, `seed`?), `keep` (`spec`?, `on`?: the ★), `drums` (`on`?), `load` (`deck`, `id` 3A or `group` + `side`, `spec`?: a pad: playing, bar-locked to the other deck), `load_pair` (`group`, `spec`?: the A + B pad), `open` (`on`?). Naming `spec` on a load picks it first. `state section=loopsets` is the status |
| `render` | MCP only (`djapp_render`, run in the MCP server's process, not the socket): an offline render of an arrangement or the action log (docs/HEADLESS.md) |
| `windows` | the top bar's buttons: `list` (default), `open` / `close` / `toggle` (`window`: td3 mpk kits bindings log wheel loops perform effects aux net heos menu map midi route vocoder browser perf set tracks mashup record live; `on`; `inserts` still names the Effects window, where the inserts now are), `closeall`. The same flags as `config panels.*`, for every window |
| `menu` | the ≡ Menu window: `status` (default), `keep_awake` (`on`), `toybox` (the Install / Update button; writes a SoundFont file, sf2 builds) |
| `controllers` | controllers and Mixxx mappings (docs/MIXXX-MAPPINGS.md): `status` (default: the mapping folders, how many mappings, every input port and what drives it: a hand-made profile, a Mixxx mapping or nothing and why), `list` (`filter`), `load` (`port`, `mapping`: a mapping onto a port nobody has a profile for; answers the report's counts), `unload` (`port` or `device`), `report` (`device` or `port`: every control mapped with its target, every one not with the reason), `rescan`, `map` (`port`, `mapping` or `none`: the config's `mixxx.map` override). The mappings are read from the user's own Mixxx install or `mixxx.dir`; none is bundled or downloaded |
| `analysis` | beat, tempo and bar-1 detection (docs/BEAT-DETECTION.md): `status` (default: the worker, Beat This! and its models, the settings, every grid found, the latency measurements, the log), `beats` (`target` capture / clip / input / file / library / vox, `id`, `input`, `path`, `seconds`, `force`, `trim`: starts the analysis off the audio thread and answers at once, with the grid when it is already known), `grid` (read what was found), `set` (`neural`, `capture_trim`, `min_confidence`, `snap` off / beat / bar, `history_auto`, `history_seconds`), `models`, `fetch_models` (about 11 MB, checksum-verified; needs a `neural` build), `retry`. Every grid names the `analyzer` that found it, pinned like `gen@N`: `beatgrid@1` is the DSP detector, `beatgrid@2:<SHA-256 of the model>` is Beat This!. The cache is keyed by the audio and the analyzer, so a changed detector or model analyses again and never answers with an old grid. `state section=analysis` |
| `wheel` | the Wheel window: `status` (default: the instrument and its motion and sound, `playing`, `busy` while a worker winds a change, the meters), `presets`, `play` / `pause` / `toggle`, `load` (`preset`), `motor` (`on`, `speed`), `interval` (`seconds`), `pluck` (`tube`?), `level` (`db`), `trigger` (`on`, `pull`), `key` (`root`, `transpose`), `brightness` (`value`), `sequence` (`notes`). The strip's mixer knobs are `input input=Windwheel`. `state section=wheel` |
| `inserts` | the inserts, in the Effects window (no window of their own since the merge; `target` is the Effects window's aim): `status` (default; `target` A / B / Wheel / Master), `on` (`effect` filter / crush / delay / reverb, `on`), `set` (`param` e.g. `delay.mix`, `value` 0..1), `off` (every insert on every target off: the inserts half of the window's ALL OFF, which also ends every target's throws). Kept in inserts.json as the window keeps them. `state section=inserts` |
| `effects` | the Effects window: `status` (default), `catalogue`, `tap` / `hold` / `release` (`id`: a pad), `fire` (`id`, `bars` 1-64 or `latch`: a pad by id for that long whatever the window's length; a transition takes none and runs its own, a riser's `bars` are its sweep, a held one-shot is let go with `release`; what a program's `macro` line calls, docs/beatscript/ext-control.md), `length` (1-4 or latch), `sweep` (`bars`), `knob` (`value`: the filter), `panic` (ends the target's throws and switches its inserts off, tails ringing out), `target` (aims the inserts too); each takes `target` A / B / Wheel / Master (default the window's aim) |
| `moves` | the Perform window's DJ moves: `status` (default), `arm` / `cancel` / `toggle` (`kind` bass_swap / phrase_sync / filter_exit / echo_out / loop_roll / vocoder_handoff, `deck`, `bars`, `do` seed / cut), `cancel_all` (docs/PERFORM.md) |
| `status` | version, uptime, output line / device / rate / level, each input strip's device (or why it failed), gear present (inpulse, apc, mpk, td3, pd705) with ports, controllers driving, the clock, counters, notes, TD-3 log |
| `state` | `section`: all, decks, mixer, transport, loops, loopsets (the Loops window), inserts, wheel, inputs, vocoder (the selected instance), vocoders (every instance in use), tricks, autonext, autodj, moves. Each deck has `sound`: kit, palette, role voices, the net macros in effect (and the roles each holds), the walk, sample auto-gain |
| `audio` | `devices` · `output` (`device`: part of a name, or null for none) · `input` (`input`, `device` or null to disconnect, `channel` L / R / LR) |
| `deck` | `deck` A / B, `action`: state, play (`on`), pause, toggle, stop, cue, load (`genre`, `seed`, `structure` or `spec`; `spec` `render:<cid>` or `render:<cid>[a..b]` puts the offline render of a public CC0 program on the sample deck, whole or its bars a to b counted from 1, with the grid the program says and no beat detection; a program that is not CC0 and public, or uses samples, is refused), demo, file (`path`), next, seed_step (`delta`), tempo (`bpm`), pitch / pitch_range (`value`), nudge (`ratio`, `duration`: the record of how long), sync (`align`), section (`index`), hot_cue / clear_cue (`slot`), loop (`op` in / out / halve / double / exit / beats; on a sample deck's grid, and on a net deck too: its loop is a window of the net's steps that plays the same each pass), jump (`beats`, negative is back: a beat jump that keeps the deck's place in the beat; a sample deck moves by its grid, a net deck in whole steps (a quarter beat), on its next bar line while it plays; refused with why when there is no grid or the jump is past the song's end), seek (`seconds`, `from_seconds`: where it was, for the record), scratch (`seconds`: the net turn of the platter, `beats`, `strokes`, `duration`, `position`: a record of a jog gesture, as djrust's activity tail writes it; it replays as the net turn), role (`role`, `on`), macro (`id`), master_macro (`id`, `bars`), panic, rewind, focus, sound (`palette`: an id or null = the default, or `step` ±n; neither lists the palettes: the whole track from a palette at the next bar, same song, kept for the deck's new tracks, logged as `setPalette`; also the deck panel's SOUND picker and the bindable `soundPrev` / `soundNext` targets), copy (`from`, `to`, `start`: docs/COPY-DECK.md; by default the other deck onto this one, on its next bar in sync; `start=top` loads the net from its start) |
| `perform` | any `DeckAction` as session JSON (`{"op":"param","deck":"A","name":"eqLow","value":0.3}`), through `Engine::perform`: answers done / stored / ignored / deferred |
| `transport` | state, start, stop, toggle, tap, lock (`deck`), unlock, free_bpm (`bpm`), auto (`on`: true / false / toggle, default toggle). The state has `auto` {on, held}: a start / stop / toggle / lock / unlock here holds auto off until a deck next plays (docs/HUB.md, "Auto transport") |
| `mixer` | `crossfader`, `master`, `headphone_mix`, `headphone_level`; `deck` (A / B / L) with `volume`, `gain`, `filter`, `low`, `mid`, `high`, `pfl`, `mute`, `aux` (its aux send) |
| `fx` | set (`param`, `value`), kill (`on`), reset, params; `deck` or both. BeatScript says the drive as a word: `deck A: drive 0.6` is `fx set deck=A param=drive value=0.6`, `master: drive 0.6` the same with no deck (docs/BEATSCRIPT.md "Control") |
| `aux` | the aux send/return bus and the macros over it (docs/AUX.md). `strip` (A, B, L or an input's label) with `send` 0..1; `return` 0..1, `mute`; any of beats' six sliders by id (`reverb-size`, `reverb-damp`, `reverb-wet` 0..100, `delay-time` 1..100, `delay-feedback` 0..90, `delay-wet` 0..100); `reset` (the sliders); `macro` (any macro id: the net and master ones, and the eleven pan / shape / aux-FX macros `ping-pong`, `hard-left`, `hard-right`, `auto-pan`, `mono`, `delay-throw`, `dub-delay`, `cathedral`, `tighten`, `loosen`, `pulse`) with `deck` (default the focused one) and `bars`: fired on the deck's next bar, logged as an `aux` live op; `cancel` (`deck`, else all). Answers the bus: `sends`, `return`, `mute`, `fx`, `running`, `peak`. No params: just reads it |
| `input` | `input` and any of `trim_db`, `volume`, `mute`, `pfl`, `filter`, `low`, `mid`, `high`, `channel`, `latency_ms`, `xfade` (A / B / off: the strip on a side of the crossfader, as a deck) · verbs: `status`, `calibrate_latency` (`seconds`, `apply`, `min_confidence`: how late a source slaved to the master clock arrives, off the audio thread; docs/BEAT-DETECTION.md), `latency` (the last measurements). Also named `inputs` |
| `loops` | list, play / stop (`id` or `slot`; play only plays, stop only stops: answers `changed`, `state`), to_deck (`deck`, `id` or `slot`, neither = the newest clip), library_to_deck (`id`, `deck`), stop_all, remove, record (`source` A/B/V/M/an input label, `on`, `bars`, `pre`), grab (`source`, `bars`, `pre`; docs/LOOP-SOURCES.md), volume, mute, library, library_load / library_save / library_delete, library_row, root (`op`: status / set `pitch` C, F#, Bb or 0-11 / detect / auto `on`: the pitch class keys transpose a loop from; docs/MPK.md) |
| `capture` | grab (`beats`), start, end, arm (`on`), status, bars (`bars`, `anchor` start / end: a loop of exactly N bars cut on bar lines of the input's beat grid from the history head; waits a moment for a grid if there is none) |
| `history` | any strip's history (`strip` or `input`: an input's label, A, B, L, V, M; docs/INPUT-HISTORY.md, "Any strip"): status, touch, scrub (`beats` or `seconds`: negative goes back in time), release, live, loop (`beats`), loop_step (`dir`), beats (window), jog (`deck` A / B / off), snap (`deck`?: the jog's history back to live) · with a beat grid (an input's, docs/BEAT-DETECTION.md): grid (tempo, bar 1, confidence, the bar lines in the window and the head's nearest), align (`unit` bar / beat, `steps`: the head onto a line), loop_bars (`bars`: loop exactly N bars in place, ending on a bar line) |
| `clock` | `td3_clock`, `td3_offset_ms`, `pd705_steps` (as td3's), `pd705_kit` (factory / acid-house), `pd705_loop_pads` (1-15), `td3_source_auto`, `td3_source` (usb / internal / read) |
| `td3` | `pd705_steps` (`on` / `off` set it, `state` reads it; with any action, so `djctl app td3 pd705_steps=on`: the PD705's groove on the clock, also `djapp --pd705-steps`), status, keys (`on`), octave, channel, generate (`seed`, `root`), slot, step (`step` 0-15 with `note` C…B / C^ or 0-12, `transpose` down / none / up, `accent`, `slide`, `tie`, `rest`; or `active` 1-16: the window's step grid), audition (`step`), write (a dry run unless `confirm: true`), cutoff_cc (`cc`, or null: the CC a `td3_cutoff` trick rides; the stock TD-3 has none) |
| `kits` | The PD705 kit library (docs/PD705-KITS.md). `action`: status (default), list, get (`kit`), add / remove (`kit`), rename / duplicate (`kit`, `to`), select, host (`host`: the Pi), pad (`pad` 1-15 / pad7 / trig1 / foot2 with `file` or `clip` / `slot` (a loop-row clip, exported to a WAV), `name`, `volume`, `loop` true / false / auto, `clear`), pattern (`op` get / step (`pad`, `step` 0-based, `velocity`, 0 clears) / cycle / bars (1-4) / preset (house, breaks, halftime, random + `seed`, empty) / default / set (`pattern`)), build, install (a **dry run** unless `confirm: true`; refused while the hub plays the pad unless `confirm_live`; `wait: true` holds up to 30 s), loaded (the staged kit is on the pad), mark_loaded, forget (`all`), track (`on` / `off` / `state`), audition (`pad`), open (`on`, `tab` pads / steps) |
| `midi` | ports, monitor (`port`, `n`; `paused` on / off and `clear`: the monitor window's Pause and Clear), send (`device`, `bytes`: channel voice or Start / Continue / Stop only, never SysEx), out (`device`, `args`: a `midiOut` binding's args) |
| `bindings` | devices, list (`device`, `group`), map (`device`), preset (`preset`), preset_add (`device`, `name`: a copy of the live preset, made live) / preset_delete (`device`: the live one), bind (`device`, `binding`), unbind (`device`, `group`, `index`, `shift`), press (`device`, `group`, `index`, `shift`), targets (`family`) |
| `route` | loop routing (docs/LOOP-ROUTING.md): status, set / toggle (`id` or `slot`, else every loop playing; `part`, `dest`, `on`), clear, notes (`id`/`slot`, `part`, `n`), offset (`ms`), add_dest (`dest`), open (`on`) |
| `log` | the action log and its arrangement (docs/ACTION-LOG.md): lines (`n`, `since`), arrangement / save (`from_bar`, `to_bar`, `path`), load (`path`), play (`path`, `arrangement` or `from_bar`+`to_bar`), stop, clear, open |
| `post` | the post chain (docs/POST.md). Live, on the master before the master level, **off by default**: `status` (default: `on`, `preset`, `chain`, `running`, the leveler's `gain_db` / `short_term_lufs`, `limiter_db`, `noise_dbfs`, `latency_ms`, `presets`) or `set` (`on`, `toggle`, `preset`, `chain`, `step` ±1; any of them alone means set), logged as a `post` live op. Files, answered on the socket's thread (the window never waits): `process` (`in`, `out` .wav / .mp3, `preset` or `post_chain`, `float32`), `measure` (`in`), `presets`. Each file result has `before` / `after` LUFS, LRA, true peak and noise floor |
| `target` | `id` (any registry target: captureGrab, historyScrub, td3ClockSource, midiOut, mapPreset, oneShot, …) with `deck`, `slot`, `index`, `value`, `pressed`, `release`, `args` |
| `keys` | where the controllers' piano keys go (docs/MPK.md): status (default), mode (`mode`: live / transpose / td3 / toggle), deck (`deck`: A / B / aimed / next), quant (`quant`: now / beat / bar), scale_lock (`on`), note (`note`, `velocity`, `down`, `deck`, `quant`: one key as a controller makes it, logged as a replayable `keys` op), off |
| `pads` | the MPK's pads (docs/MPK.md): status (the mode seen, PROG / NOTE, the notice, the notes in force), learn (record the 16 pad notes as pressed and write them into the device map), cancel, clear, set (`channel`, `notes`), feed (`bytes`) |
| `vox` | the vocal pads (docs/MPK.md): status (folder, page, the 16 pads' files), dir (`dir`), scan, page (`op`: next / prev / first), set (`choke`, `stretch`, `quant`, `bpm`), play (`pad` 1-16) |
| `screenshot` | `path` (default `/tmp/djapp-<pid>.png`), written on the next frame. The window stays open |
| `clips` | the loop row in full. list (default): the row's settings (quantum, fade, follow, key lock, FX link, clock bpm), each clip's own key lock / quantum / fade / follow (null: the row's), FX, sequence steps, colour, state, the scenes, the clips removed. Shortcuts for the session `DeckAction`s, params passed through: scene_save / scene_launch / scene_rename / scene_delete (`name`, `to`), crop (`id`, `from`, `to` beats), sequence (`steps` `[{clip, repeats}]`, `replacing`), retune (`id`, `voices`), fx (`id`, `fx`), fx_link, key_lock / row_key_lock, quantize / row_quantize (`beats`), fade / row_fade (`fade` off / 10ms / beat / bar), follow / row_follow (`mode` source / louder), gain, rename, color, restore (`at`), forget |
| `wander` | status (each deck's walk: on, pace, paused, step, locks, countdown, idle reason), on / off (`deck`), pace (`bars`), lock (`dimension`: genre / tempo / key / structure / instruments / arrangement / fx / palette / layers, `on`), grab (as a turn of the browser wheel) |
| `automix` | status (state and the mixer's status line), set (`on`, `bars`, `every`), off, now, defer (`handoff`) |
| `autonext` | auto-next every N bars (docs/AUTO-NEXT.md): status (default: every deck's `on`, `bars`, `bars_left`, `pending`, `steps`, `idle`; and the presets), on / off / toggle / set (`bars`) / up / down on `deck` (default: the focus), `bars` 1-1024 with any. `state` has an `autonext` section and each deck an `auto_next_bars` |
| `autodj` | Auto-DJ (docs/AUTO-DJ.md): every `bars` bars (8-64) on a phrase line it hands over: a loop-set pair to group + 1 (wrapping), a two-deck mix by bass swap + crossfader cut, or one deck to its next seed, with the transition's effects (`style` auto / filter / echo / roll, `fade` 1-8 bars, `effects`). status (default: the settings, `state` idle / waiting / transition, `idle` reason, `bars_left`, `word`, `transitions`), on / off / toggle / set (`bars`, `fade`, `style`, `effects`, `on`) / next (one transition on the next bar line, even while off) / cancel. Config `autodj.*`; default off. `state` has an `autodj` section |
| `setlist` | set lists (docs/SETLIST.md): a set is segments (a loop-set pair `dnb:82819#3`, a net seed `net:house:42` or a saved track `track:<cid>`) with `bars`, a `transition` (cut, bass_swap, filter_exit, echo_out, loop_roll, phrase_sync, vocoder_handoff), `inserts` (filter_sweep / delay_throw over the last N bars) and a `tag`; canonical JSON with a content ID, kept in the app-support folder. `list` (default), `get` (`id`), `new` (`name`, `set`), `generate` (`segments`, `genres` as "dnb then jungle", `bars`, `vary`, `inserts`, `seed`, `name`, `save`), `save`, `load`, `delete`, `duplicate`, `edit` (`op`: insert / remove / move / bars / transition / inserts / tag / source / rename, `index`, ...), `license` / `visibility` (`value`, `author`; `id` reads a saved set's), `fork` (`id`, `name`, `author`), `share` (refused when private; docs/LICENSING.md). Every write answers the working set |
| `track` | saved tracks (docs/LICENSING.md): what a deck plays kept as a canonical, content-addressed object with its rights inside: `license` CC0-1.0 (default) / CC-BY-4.0 / all-rights-reserved, `visibility` public (default) / private, `author`, `forkedFrom`, `credits`. `list` (default), `get` (`id` or `deck`), `save` (`deck`, `name`, `license`, `visibility`, `author`), `license` / `visibility` (`id`, `value`: a new CID), `fork` (`id`, `name`, `author`: keeps the license, credits a CC BY parent's author, warns on all-rights-reserved), `share` (`id`; refused when private), `load` (`id`, `deck`), `delete` |
| `queue` | the set queue (docs/SETLIST.md): `status` (default), `add` (`id`, or the working set), `clear`, `play` (`from`), `pause`, `stop` (an echo out), `next`, `jump` (`n`, or `set` + `segment`), `loop`. Plays the queued sets back to back on the phrase grid with the Perform moves and Master inserts they hold; `state section=queue` is the same status |
| `btw` | the bitwrap language, BeatScript (docs/BEATSCRIPT.md; `live` and `livecode` are aliases): programs of pattern lines and control lines; `eval` `check` `get` (default) `save` `load` `list` `stop`, `fmt` (the canonical form the CID is of), `share` (a self-sealing link) and `open` (decode, recompile, recompute the CID, refuse a mismatch), `fork` `license` `visibility` (licensed .beatscript objects), `export` and `import` (.beatscript files), `instruments` (every voice, kit and palette the engine has), `jsonld` (the program as linked data: JSON-LD with its CID, genre, tags, key, tempo, sections, licence and Euclidean rhythms), `terms` (the bitwrap ontology's vocabulary the `#! genre:` / `#! tags:` header lines are checked against; `save` takes `genre` and `tags`), `generation` (the CID of generation N of an `evolve`, from generation 0 and the rule alone), `test` (judges the program's `assert` statements: the static ones re-checked, the `level` ones measured on an offline render; refused while a program is running or staged, because the render would hold the window and the bar clock: `stop` it, or use `djctl btw test FILE`), `perform` (the performed inputs `knob(3)` `pad(5)` `fader(A)` `held(key)` the program reads and the performance it has logged; `do` start, stop, save, list) and `replay` (a saved performance played back). Lines are `do <noun> <verb> key=value` calls checked against the registry, with sugar (`vocoder set mix=0.8`, `freeze phrase bars=2`, `vocal create "words"`), time words (bars and beats), ramps with shapes and lfos, arithmetic and `when` guards, `let` `def` (with typed ports and `wire`) `repeat` `section` `arrange` (with `stack` / `then`) `import`, and `duck` / `mix` / `sing` / `assert` lines; a song ends after its last section (`end`, `end at bar N`, `loop`), and `get` says `ended`. A program cannot call the verbs that act on the program that runs (`do btw eval`, `stop`, `load`, `open`, `replay`, `perform` are errors at `check`); a verb in the lists with no implementation is an error, never a `stop`. The Live Code window (`windows open window=live`) holds the same text |
| `trick` | performance tricks (docs/TRICKS.md): status (default: every deck's), on / off / toggle `kind` on `deck`, with `quant` (now, 32nd, 16th, 8th, beat, bar, or beats) and the kind's args (`div`, `rise`, `amount`, `bars`). `state` has a `tricks` section |
| `freeze` | granular freeze on any strip's history (`strip` or `input`: an input's label, A, B, L, V, M) (docs/INPUT-HISTORY.md, "Freeze"): status, set, on / off / toggle (`now` skips the beat), scrub (`by` beats of the frozen audio), `quantize`, knobs `beats`, `size`, `density`, `spread`, `jitter`, `position`, `mix`, `knob`+`value`. Each input in `state` has `freeze` · phrase (`bars`: freeze whole bars of an input's beat grid, ending on the next bar line) |
| `vocoder` | docs/VOCODER.md. `instance` (0-3; default the selected one; naming one not in use adds it) picks the vocoder the call acts on; `action` list / add / remove (`instance`, not 0) / select (`instance`) manage them and return `vocoders`. `modulator` and `carrier`, each any of: none, `synth`, `input:LABEL`, `deck:A` / `deck:B` (live; not the other side's deck, and the synth on one side only), `clip:ID`, `slot:N`, `library:ID`, `capture:LABEL:BEATS`; `swap` (true: exchange them), knobs in their units (`bands`, `shift`, `attack`, `release`, `mix`, `noise`, `volume`, `vowel` 0-4 or a/e/i/o/u, `vowel_mix`, `synth_level`, `synth_detune`, and the synth voice's `osc1_wave`…`osc3_level`, `pwm`, `sync`, `ring`, `cutoff`, `resonance`, `env_amount`, `env_decay`, `synth_attack`, `synth_release`, `drift`; waves by name), `knob`+`value` (0-1 travel), switches (`on`, `insert`, `mute`, `pfl`, `bypass`, `freeze`: hold the bands, `xfade`: the crossfader morphs carrier ↔ vocoded ↔ modulator), `preset` (load a synth preset), `voice` (the synth sings: a formant voice preset, or `off`; a source spelled `voice` / `voice:NAME` is that singing synth, docs/VOICE.md), `notes` (numbers or names: `[45,"C3"]`, or from a shell `notes=45,48,52,57` / `notes=A2,C3,E3,A3`; a word that is not a note is an error), `note`+`down`; `action` status / set / cycle_modulator / cycle_carrier (`dir`) / swap / preset_save / preset_delete (`name`) / preset_next / preset_prev. `state` has a `vocoder` section |
| `vocal` | docs/VOICE.md: typed lyrics, sung on pitch by the formant voice (CC0 by default; phonemes from the BeatScript lexicon (CC0) or `/…/` escapes + letter-to-sound rules, no espeak-ng). `action` status (the current line, the voices, where the phonemes come from) / words (`lyrics`: each word's phones, whether the dictionary or the rules gave them, and the syllables as sung) / create (`lyrics`, `key`?: Bb minor, `bpm`?: the master's, `bars`?: 2, `preset`?: robot, `seed`?, `name`?, `license`?, `visibility`?, `author`?, `notes`?: your own, `start:dur:note …` or `[{start,dur,midi}]`, one per syllable; at most sixteen syllables a bar) / get (`id`?) / list / save (`name`?, `license`?, `visibility`?) / load (`id`) / license, visibility (`id`?, `value`?: a new one is a new CID) / fork (`id`?, `name`?, `author`?: credits its parent) / share (`id`?; a private line is refused) / delete (`id`) / play (the headphone cue once; `stop`) / to_row (`play`?: the loop row, exactly its bars long) / to_deck (`deck`) / to_vocoder (`role`: modulator · carrier) / hook (`lyrics`, `deck`?, `key`?: fitted to a deck track's key at the master tempo, looping on the loop row) / render (`path`, `sample_rate`?, `seconds`?: a WAV). Alias `lyrics` |
| `voice` | docs/VOICE.md: the formant voice, robot and choir vocals with no recorded singing. `action` status (presets, vowels, onsets, each deck's sung nets, the vocoder's voice) / presets / vowel (`deck`?, `vowel`: a e i o u ah oo or 0-6, `net`?: the vowel a deck's voices sing, live) / load (`deck`?, `genre`?, `seed`?, `style`: choir · robot · auto, `preset`?, `structure`?: a track with a sung `vox` part) / vocoder (`preset` or off, `carrier`?: the vocoder's synth sings). Alias `formant`. Not `vox` (the vocal sample pads) |
| `heos` | docs/HEOS.md. The Denon HEOS zones: status (default: zones, volume, mute, state, now playing, groups, the stream), volume (`zone`, `level` 0-100 or `step`, `max`), mute (`zone`, `on`), play / pause / stop, group / ungroup (`zones`), stream (`zones`, `format` flac / wav: the master over HTTP, several seconds behind the booth), stream_stop, stream_server_stop, select, discover, connect (`host`), refresh, open |
| `perf` | the window's perf overlay (off by default; F3 or the status bar's `perf` in the window). `action`: status (default), on, off, toggle, reset (count from now), set (`target_hz`: the display's rate; 0 estimates it from the median frame interval). Answers `fps` (frames painted in the last second), `display_hz`, `continuous` (drawing every display frame: a deck plays or the overlay is up, and the window is visible), `frame` (interval mean / p50 / p95 / max, `work_ms` inside `ui`, `cpu_ms` eframe's own including tessellation and paint), `late_frames` (> 1.5 display periods) and `dropped_frames`, `window` (visible, focused, occluded), `audio` (the output's buffer, longest callback gap, `underruns` = backend xruns + silent fills + callbacks over twice their buffer late) and `platter` per deck while it plays (`raw_updates_per_s`: how often the published playhead moved, `raw_still_pct` / `drawn_still_pct`: frames on which it and the drawn angle stood still, `max_step_deg`) |
| `config` | the config file (below): get (`key`: a key or a section; none: all), set (`key`=`value` pairs, or `key` + `value`, or `values: {…}`), diff (`against`: defaults / file / launch), save, reload, keys, path |
| `net` | docs/NET-PROTOCOL.md (`network` builds): status (default: the host, port, `pair_code`, trust, discovery, the peers with their clock offset, round trip, BPM and phase against the master, the paired list, the log), track (`deck`, `cid`, `payload`, `play`, `sync`: a beats track by CID, checked and composed here), glance (the decks, mixer and stream, compact), stream (`url`: the phone's `/master.djl` into the `Phone` strip) / stream_stop / stream_status (state, BPM, phase against the master, the latency figures, drift correction, drop-outs, reconnects), aux_sync, aux_lock (`on`), aux_nudge (`beats`), aux_trim (`ms` ±50), aux_latency (`ms` 30-1000), aux_match (the master deck onto the stream's tempo), code (a new pairing code), trust (`trust` code / open / off), forget (`id` or `all`), open. The same method answers the phone over djnet, less the pairing code, peers and log |
| `key` | docs/KEY.md. `status` (default: each deck's key with its `source` track / tag / audio / detecting, Camelot, `key_lock` (a net deck's is inherent), the BPM, and `pair`: `compat` same / relative / adjacent / diagonal / boost / parallel / clash, `hint`, and when they clash `suggest` {semitones, key}), `detect` (`deck`?, `profile` ensemble / krumhansl / temperley / albrecht), `set` (`deck`, `key` or null: a hand tag on what the deck holds), `lock` (`deck`, `on`), `tempo` (`deck`, `bpm`: key-locked on a sample deck by a stretched copy swapped in at the same place), `compat` (`a`, `b`) |
| `mashup` | docs/MASHUPS.md. The working mashup: `status` (default: sources with key and BPM, parts, `target_key`, `clashes` with `suggest_transpose`, `cid`, `parents`, the stem sources), `sources`, `new` (`name`), `add` (`spec` genre:seed / genre:seed:B / deck:A / audio:<cid>, `label`, `key`, `bpm`), `remove` (`label`), `stems` (`source`), `assign` (`slot`, `source`, `stem`), `part` (`slot`, `level`, `mute`, `solo`, `transpose`, `remove`), `key` (`key` or null, `auto`), `tempo` (`bpm`), `play` (`deck`, `id`), `save` (`name`), `load` / `delete` (`id`), `list`, `fork` (`name`), `open` (`on`) |
| `record` | docs/RECORDING.md. `status` (default: `recording`, `seconds`, `frames`, `dropped_frames`, `from_bar`, the live `tracklist`), `start` (`name`, `max_minutes`), `stop` (`name`: answers the saved set with its `cid`, `signed_by`, `tracklist`), `cancel`, `mark` (`kind` track / mashup / loopset / transition, `label`, `cid`, `deck`), `list`, `get` / `tracklist` (`id`; default the newest, or the live one), `program` (`id`, default the newest: answers `text`, the set written out as a BeatScript `set` of `play` and `mix` lines, its `program_cid`, and `take_cid`, the take that names that program by CID; `beatscript set-program` writes both as files), `verify` (`id`: `cid_ok`, `audio_ok`, `signature_ok`), `replay` (`id`), `fork` (`id`, `name`), `delete` (`id`: the whole cid) |
| `browse` | docs/BROWSER.md. `action`: status, packs (`source`), list / search (`query`, `kind` vocals / drums / bass / synth / fx, `source` captures / recent / music / apple-loops, `pack`, `offset`, `limit`), load (`path` or `index`; `to` A / B / row / row_play / vocoder), audition (`path` or `index`; on the cue only), stop, gain (`value`), open (`on`), rescan, select |
| `batch` | several commands as one request (below, "Batches"): `run` (`commands`: `[{method, params, delay?}]`, each in any form the language takes; `atomic`, `stop_on_error`, `at` now / next_beat / next_bar / next_phrase / bar:N, `deck`, `label`, `dry_run`), `status` (default; `id`: one batch with its commands' results), `cancel` (`id`). A JSON array on the socket is a batch too |

`input` takes a strip's label (`TD-3`, or part of it in any case) or its index. Without one,
it means the strip a deck's jog is aimed at, or else the first strip with a history.

### What the socket can't do

- **Change the sample rate or add input strips.** The engine is built at one rate, with the strips
  `--input` gave it. `audio output` refuses a device that can't run at that rate (it doesn't
  resample), and `audio input` re-routes an existing strip. For a new rate or another strip,
  restart djapp with other flags.
- **Write a TD-3 pattern, or switch its clock source by accident.** A real pattern write needs
  `confirm: true`. Clock-source SysEx goes only through `clock td3_source`. `midi send` refuses
  any SysEx.
- **Edit the binding map without saving.** `bindings bind / unbind / preset` (and a `mapPreset`
  target) save the device's map to `<support>/maps/` the same way an edit in the window does.

## Batches

`batch` runs several commands as one request: all or nothing, and landing together on the bar
grid. The scheduler is `djhost::batch` (pure, tested on a fake clock); the window runs what comes
due through its own `control_call`, so a batch is exactly the calls it holds and nothing more.

```json
{"method":"batch","params":{
  "commands":[ {"method":"mixer","params":{"deck":"A","mute":true}}, … ],
  "atomic":false, "stop_on_error":true, "at":"now", "deck":"A", "label":"…", "dry_run":false }}
```

**Commands.** An ordered list of `{method, params, delay?}`, each in any form the language
takes: `{"method":"deck","params":{"action":"play","deck":"A"}}`, params beside the method
(`{"method":"deck","action":"play","deck":"A"}`), or dotted (`{"method":"deck.play","params":{"deck":"A"}}`).
An `id` or `jsonrpc` key is ignored. At most 256. A batch cannot hold a batch, `render` (MCP only)
or `post`'s file actions (they run on the socket's own thread).

| option | meaning |
|---|---|
| `atomic` | every command is checked against the registry first (the noun, the verb, its required params); if any is invalid **none runs** and the error lists them all. Then every command runs in **one service pass** of the window. No per-command `delay` (they would not land together). A command that fails at run time (`no deck Q`) does not stop the rest unless `stop_on_error` is set: it is reported |
| `stop_on_error` | stop at the first failed command; what would have come after is `skipped`. Default **true**, false when atomic |
| `at` | when it lands: `now` (default), `next_beat`, `next_bar`, `next_phrase` (the next 8-bar line) or `bar:N` (a bar of the grid deck's own numbering, bar 0 its start, as `log` and the queue count; a bar already begun is refused) |
| `deck` | the grid deck to time on (default: the first of A, B that plays) |
| `delay` (per command) | beats (`2`, `"2 beats"`) or bars (`"1 bar"`) after the landing, for a short sequence such as a filter sweep in steps. At most 16 beats; anything longer is a set list (docs/SETLIST.md) or the live-code language |
| `dry_run` | validate and report, run nothing (below) |
| `label` | a name, shown by `status` |

**Timing** reuses what the Perform moves, Auto-DJ and the queue use: it counts the grid deck's bar
lines (a loop's wrap or a swap to the top is a line too, a small step back is jitter) and lands on
the service pass that crosses the line, ~16 ms after it, in the same pass as the moves. With no deck
playing there is no grid: a timed `at` lands at once (the batch says so in `note`), and a `delay`
counts beats at 120 BPM. If the grid deck stops while a batch waits, the line does not come until
a deck plays (or the batch is cancelled); if the grid moves to the other deck the count restarts
there. `delay` counts the grid deck's tempo from the landing.

**Answer.** `{id: "b1", state, label?, at, atomic, stop_on_error, commands, ran, failed, ok?, results}`.
`state` is `done`, `pending` (waiting for its line: `waiting_for` and `lands` say which), `running`
(landed, with delayed commands to come) or `cancelled`. Each result is `{index, method, status, ok,
result | error | skipped}`: `status` is `ok`, `error`, `skipped` or `waiting`, and `result` is
the state the command answers alone (a write answers its noun's state after it). A batch that lands
now with no delays is `done` in the answer; a timed one answers `pending` at once (no connection is
held), and `batch status id=b1` reads its results when it has landed. `batch status` (no id) lists
the batches waiting and the last 16 that finished; `batch cancel id=b1` stops what has not run
(a batch already running keeps what it did; the rest are `skipped: cancelled`). At most 16 batches
wait at once. They are in memory: they do not survive a restart of djapp.

**Dry run.** `dry_run: true` validates every command and runs nothing. It says whether the batch
`valid`, how many commands `would_run` (all or none when atomic; up to and with the first invalid
one when `stop_on_error`), when it `lands` (or why it cannot: `landing_error`), whether any command
is `live` (reaches the speakers, gear, network or disk: ask first), and for each command its
verdict, `effect` and `doc`. For a write, `changes` lists `{param, from, to}` where a param's
name is a scalar key of its noun's state now (read through the noun's own status / state): best
effort, since a state does not hold every param by its name, and a command that depends on an
earlier one is compared with the state before the batch.

**The array form.** On the socket a line that is a JSON array is a batch with `stop_on_error` off
(JSON-RPC's batches are independent calls), run in order in one pass of the window, and answered
as an array in the same order, each with its own `id`:

```
→ [{"id":1,"method":"mixer","params":{"deck":"A","mute":true}},{"id":2,"method":"deck.state","deck":"A"}]
← [{"id":1,"result":{…}},{"id":2,"result":{…}}]
```

A request that is not an object with a `method` is answered `{"id":null,"error":…}` in its place; an
empty array, or a djapp without `batch`, is one `{"id":null,"error":…}` object. A request may carry
a `delay`; it then answers `{"pending":true,"batch":"b1"}` and the batch runs on. `at`, `atomic` and
`dry_run` need the `batch` noun.

**Examples.** Mute three strips atomically (none runs if any is misspelt, and all three change in
one pass):

```json
{"method":"batch","params":{"atomic":true,"commands":[
  {"method":"mixer","params":{"deck":"A","mute":true}},
  {"method":"mixer","params":{"deck":"B","mute":true}},
  {"method":"input","params":{"input":"TD-3","mute":true}} ]}}
```

On the next bar, load a pair, open the Master filter and turn the delay on (it answers `pending`
until the line; read it with `batch status id=b1`):

```json
{"method":"batch","params":{"at":"next_bar","atomic":true,"label":"drop","commands":[
  {"method":"loopsets.load_pair","params":{"spec":"techno","group":2}},
  {"method":"inserts.on","params":{"target":"Master","effect":"filter"}},
  {"method":"inserts.on","params":{"target":"Master","effect":"delay"}} ]}}
```

A filter sweep in four steps, a beat apart from the next bar:

```json
{"method":"batch","params":{"at":"next_bar","commands":[
  {"method":"inserts.set","params":{"target":"Master","param":"filter.sweep","value":0.25}},
  {"method":"inserts.set","params":{"target":"Master","param":"filter.sweep","value":0.5},"delay":1},
  {"method":"inserts.set","params":{"target":"Master","param":"filter.sweep","value":0.75},"delay":2},
  {"method":"inserts.set","params":{"target":"Master","param":"filter.sweep","value":1.0},"delay":3} ]}}
```

**What "atomic" does and does not promise.** The registry check is complete before anything runs,
and the commands then run back to back on the window's thread: no other request, controller or
frame runs between them, and they reach the audio thread's command queue within microseconds of one
another. It is **not** a fence in the renderer: the renderer drains that queue once per block, so
in principle a block could start between two of the commands' messages. A hard guarantee needs a
barrier message in djrust's engine (`Msg`), which is a change to the public engine repo. A
command's own refusal at run time (a state it cannot be applied to) cannot be known before it
runs and is not rolled back: it is reported, and what ran before it stays done.

**From the shell and MCP.**

```sh
djctl app batch set.json                       # {"commands":[…], "at":"next_bar", …} or a bare array
cat set.json | djctl app batch - --atomic --at next_bar
djctl app batch set.json --dry-run             # validate and report
djctl app batch status                         # the batches; `batch status id=b1` for one
djctl app batch cancel id=b1
```

`--atomic`, `--dry-run`, `--at VALUE` and any `key=value` after the file override the file's own
options. The file may be an object with `commands`, a bare array of commands, or one command. The
MCP tool is `djapp_batch`, made from the registry like the others (its schema carries the commands'
shape); a client uses it the same way, `describe` lists its verbs and params, and its `run` is
marked **live** since a batch may hold any command: read the commands, and `dry_run` first.

## Config

One JSON file holds what a booth restart should come back with:
`~/Library/Application Support/dj-vevr-se/djapp.json` (elsewhere `$XDG_DATA_HOME/dj-vevr-se/`).
`--config PATH` picks another, `--config off` none, `$DJAPP_CONFIG` the same as `--config`.

- **Launch:** djapp reads the file, then the flags override it. A flag that lists (`--input`,
  `--input-latency LABEL=…`) replaces the file's list (or that label's entry), it doesn't add
  to it. A switch flag (`--td3`, `--midi`, …) can only turn a thing on.
- **Hot reload:** the file is read about once a second. A key whose value changed is applied
  live through the same calls as the socket's methods; a key only a launch reads is kept and
  reported (`config path` → `restart_pending`, the notes, stderr). A file that doesn't parse is
  ignored (nothing changes) and reported.
- **Write-back:** a key changed in the window, by a controller, over the socket or by
  `config set` is written back about a second after it stops moving (a fader writes once).
  Only keys that changed while the app ran are written: a launch flag's value doesn't overwrite
  the file. A key removed from the file goes back to its default.
- **One owner:** the default file is the booth's. A djapp started on another `--control` socket
  (or `--control off`) doesn't read it unless given `--config` (or `$DJAPP_CONFIG`), and one
  djapp at a time holds it (`djapp.json.lock`, a `flock`): a second one runs without it and says
  so. Tests and headless runs can't write their changes into the booth's settings.
- **Seeding:** `djapp <flags> --save-config` writes those flags into the file and exits.

```sh
djctl app config                       # every key as it runs now
djctl app config get td3               # a section (or one key: td3.offset_ms)
djctl app config set td3.offset_ms=4 panels.perf=on
djctl app config diff                  # what differs from the defaults (diff file | launch)
djctl app config save                  # every key, as it runs now, into the file
djctl app config reload                # read the file now and apply every key it sets
djctl app config keys                  # the table below, with types and defaults
```

`set` answers `applied` (done live, saved), `restart_needed` (saved, with why it needs a
launch) and `errors`; an unknown key or a bad value sets nothing.

| key | live | flag | what |
|---|---|---|---|
| `audio.output` | yes | `--device` | the output device (part of its name); null: the system default; `none`: no output. A device at another sample rate is refused (the engine's rate is fixed at launch) |
| `audio.inputs` | re-route | `--input`, `--no-inputs` | `["DEVICE=LABEL", …]`; null: the plugged-in defaults. The same labels in the same order re-route live; adding, removing or renaming a strip needs a restart |
| `audio.input_channels.LABEL` | yes | | L, R or LR |
| `audio.input_latency_ms.LABEL` | yes | `--input-latency` | captures read this many ms late |
| `audio.cue_pairs` | restart | `--cue-pairs` | `"M,C"` |
| `audio.sf2_dir` | restart | `--sf2-dir` | the SoundFont folder |
| `audio.start_muted` | restart | `--no-start-muted` | every input strip (TD-3, PD705, Windwheel, Phone) starts muted at launch, whatever the last session left; default on. Unmute from the GUI, a controller or the socket (`input` with `mute=false`) |
| `engine.plain` | restart | `--plain` | the plain engine |
| `history.beats` | yes* | `--history-beats` | each input's history window; *turning it on (from 0) or off needs a restart |
| `history.jog` | yes | `--history-jog` | `"B=TD-3"`: that deck's jog scrubs that strip; null: off |
| `td3.clock` / `td3.offset_ms` / `td3.source_auto` | yes | | the TD-3's clock from the hub, its offset (±50 ms), the USB / internal clock-source switch |
| `td3.pd705_steps` | yes | `--pd705-steps` | the PD705's groove on the clock |
| `td3.keys` / `td3.octave` / `td3.channel` | yes | | the MPK's keys play the TD-3 (the key mode `td3`; **default off** since 2026-11-07), their octave (-3…3), its channel |
| `keys.mode` / `keys.deck` / `keys.quant` / `keys.scale_lock` | yes | | what the MPK's keys do (live / transpose / td3), which deck, when they sound, the scale lock (docs/MPK.md) |
| `vox.dir` / `vox.choke` / `vox.stretch` / `vox.quant` / `vox.bpm` | yes | | the vocal pads' folder, choke group, tempo-stretch, quantise and fallback tempo (docs/MPK.md) |
| `loops.auto_root` | yes | | find the key of each new loop (docs/MPK.md, "Loop root") |
| `controller` | yes* | `--controller` | drive the decks only from matching controllers; *to or from `none` (no MIDI) needs a restart |
| `bindings.lock` | yes | | the Bindings window's lock |
| `transport.auto` | yes | | the transport follows the decks (default on): a deck playing starts the clock locked to it, the master deck keeps it, the last one stopping stops it (docs/HUB.md, "Auto transport") |
| `transport.lock` | yes | | the deck the master tempo is locked to, by hand or by auto (the last lock comes back on a restart); null: the playing deck the crossfader favours |
| `decks.A.palette` / `decks.B.palette` | yes | | the deck's sound palette (null: the track's own), at the next bar; a sample deck keeps the saved one |
| `decks.A.auto_next` / `decks.B.auto_next` | yes | | Auto-next |
| `decks.A.auto_next_bars_on` / `decks.B.auto_next_bars_on`, `decks.X.auto_next_bars` | yes | | auto-next every N bars on / off, and N (1-1024, default 16) |
| `decks.A.pitch_range` / `decks.B.pitch_range` | yes | | the tempo fader's travel |
| `mixer.master` / `mixer.headphone_mix` / `mixer.headphone_level` | yes | | the output levels |
| `post.on` / `post.preset` | yes | | the live post chain on the master (off / `clean-medium`; docs/POST.md) |
| `panels.midi` `.td3` `.bindings` `.map` `.route` `.log` | yes | `--midi` `--td3` `--bindings` `--map` `--route` `--log` | which windows are open |
| `panels.vocoder` `.browser` `.heos` `.aux` `.perf` | yes | | the same, no flag |
| `loops.dir` / `loops.library` | restart | `--loops-dir` / `--no-loop-library` | the loop library |
| `heos.mode` | restart | `--heos` | lazy, auto, off or a host |
| `heos.port` | yes* | `--heos-port` | the master stream's port; *while the stream server is stopped |
| `control.socket` | restart | `--control` | the socket's path; `off`: none |
| `window.size` | restart | `--size` | `WxH` |

Not in the file, by design: the deck specs (`SPEC_A SPEC_B`), `--play`, `--screenshot`,
`--headless` and `--config` / `--save-config` themselves: they say what one launch does, not a
setting to keep.

## Audit: every control has a runtime path

The full table, with the MCP tool for each and what is still missing, is docs/CONTROL-COVERAGE.md. The tables here are the older, per-window notes.

Every control in the window that changes state, and every launch flag, with the socket
method (or config key) that does the same. Controls that only change what the window shows
(the map's device filter and "hide unbound", the Bindings window's device and row selection,
the browser's search box) have no path and need none.

### The window

| where | control | runtime path |
|---|---|---|
| top bar | every window's button (TD-3, MPK, Kits, Bindings, Log, Wheel, Loops, Perform, Effects (with the inserts), Aux, Net, HEOS, ≡, Map) | `windows open|close|toggle window=…` (all of them); `config` `panels.td3` / `.bindings` / `.log` / `.aux` / `.heos` / `.map` (also `log open`, `heos open`); the MPK window's settings are `keys`, `pads`, `vox`, `loops root` and the `keys.*` / `vox.*` config keys |
| banner | the MPK pads are in PROG mode / a pad learn waits | `pads` (`notice`, `learn`, `cancel`) |
| status bar | perf (and F3) | `perf on|off|toggle`, `panels.perf` |
| deck | genre / seed / structure + Compose, Random | `deck load` (`genre`, `seed`, `structure`) |
| deck | Browse…, file drop | `deck file` (`path`), `browse load` |
| deck | Demo loop | `deck demo` |
| deck | SOUND ◀ ▶ and the palette picker | `deck sound` (`step`, `palette`), `decks.X.palette` |
| deck | Copy to A/B | `deck copy` |
| deck | Play / Pause, CUE, \|< | `deck play` / `pause` / `toggle`, `deck cue`, `deck rewind` |
| deck | Phones | `mixer deck=X pfl` |
| deck | REC (hold), Grab | `loops record` (`source`, `on`), `loops grab` |
| deck | Auto-next | `config decks.X.auto_next`; `perform {op: autoAdvance}` |
| deck | Every N / Next in n/N (right-click: bars) | `autonext on|off|toggle|set|up|down`, `decks.X.auto_next_bars_on` / `_bars` |
| deck | Panic | `deck panic` |
| deck | pitch slider, 0, ±range | `deck pitch` (`value`), `deck pitch_range`, `decks.X.pitch_range` |
| deck | hot cues 1-8 | `deck hot_cue` / `clear_cue` (`slot`) |
| deck | loop In / Out / Exit / ½ / ×2 / beats | `deck loop` (`op`) |
| deck | roles | `deck role` (`role`, `on`) |
| deck | net macros, master macros | `deck macro` (`id`), `deck master_macro` |
| deck | FX kill | `fx kill deck=X` |
| deck | trick buttons | `trick on|off|toggle` |
| mixer | GAIN, HIGH / MID / LOW, FILTER, fader, mute (A, B, L) | `mixer deck=X` `gain` / `high` / `mid` / `low` / `filter` / `volume` / `mute` |
| mixer | AUX send (A, B, L) | `mixer deck=X aux`, `aux strip=X send`; the `auxSend` binding target |
| mixer | FX RET fader, mute, `Aux` | `aux return` / `mute`; `auxReturn` / `auxMute` targets; `config panels.aux` |
| mixer | MASTER, CUE MIX, PHONES | `mixer master` / `headphone_mix` / `headphone_level`, `mixer.*` |
| mixer | crossfader | `mixer crossfader` |
| mixer | transport Play, TAP, free BPM, tempo lock, AUTO | `transport toggle` / `tap` / `free_bpm` / `lock` / `unlock` / `auto`, `config transport.auto` / `transport.lock` |
| input strip | TRIM, HIGH / MID / LOW, fader, mute | `input` `trim_db` / `high` / `mid` / `low` / `volume` / `mute` |
| input strip | AUX send | `aux strip=LABEL send`, `auxSend` with `args.strip` |
| input strip | channel L / R / LR | `input channel`, `audio input channel`, `audio.input_channels.LABEL` |
| input strip | latency ms | `input latency_ms`, `audio.input_latency_ms.LABEL` |
| input strip | history beats, jog aim | `history beats` / `history jog`, `history.beats` / `history.jog` |
| input strip | freeze, its size and knobs | `freeze on|off|toggle`, `freeze set` (`size`, …) |
| input strip | LOOP / GRAB | `loops record` / `loops grab` (`source`: the label) |
| V / M buses | LOOP / GRAB | `loops record` / `loops grab` (`source` V / M, `pre`) |
| loop row | Route… / Vocoder… / Browser… | `route open`, `panels.route`; `panels.vocoder`; `browse open`, `panels.browser` |
| loop row | All out, a tile (play / stop), →A / →B, Onto deck, Off the row, save | `loops stop_all`, `loops play` / `stop`, `loops to_deck`, `loops remove`, `loops library_save` |
| FX bar | aim A / B / both | per call: `fx` takes `deck` (no state to keep) |
| FX bar | the knobs, Kill, Reset | `fx set` (`param`, `value`), `fx kill`, `fx reset` |

### The windows

| window | control | runtime path |
|---|---|---|
| TD-3 | send clock + Start/Stop, offset ms, auto, USB / internal | `clock td3_clock` / `td3_offset_ms` / `td3_source_auto` / `td3_source`, `td3.*` |
| TD-3 | PD705 steps on the clock | `td3 pd705_steps=on|off`, `td3.pd705_steps` |
| TD-3 | MPK keys, octave, channel | `td3 keys` / `octave` / `channel`, `td3.keys` / `.octave` / `.channel` |
| TD-3 | seed, root, Generate; slot | `td3 generate` (`seed`, `root`); `td3 slot` |
| TD-3 | the step grid (note, oct, acc, slide, tie, rest) | `td3 step` **(new)** |
| TD-3 | step numbers (audition), Write, dry run | `td3 audition` (`step`), `td3 write` (`confirm`) |
| Kits | kit list, New / Copy / Rename / Delete | `kits add` / `duplicate` / `rename` / `remove` / `select` |
| Kits | pads: sample, name, volume, loop flag, Hear, ✕ | `kits pad` (`file`, `clip`, `name`, `volume`, `loop`, `clear`), `kits audition` |
| Kits | steps: cells, bars, presets, default groove | `kits pattern` (`op` step / cycle / bars / preset / default / set) |
| Kits | Build, Dry run install, arm + Install on the Pi | `kits build`, `kits install` (`confirm`, `confirm_live`) |
| Kits | Loaded on the pad, Forget, hold-back follows the kit | `kits loaded`, `kits forget`, `kits track` |
| MIDI monitor | Pause, Clear | `midi monitor paused=on|off`, `clear=true` **(new)**; open: `panels.midi` |
| Bindings | lock | `config bindings.lock` **(new)** |
| Bindings | preset picker, add (copy), delete | `bindings preset`; `bindings preset_add` / `preset_delete` **(new)** |
| Bindings | target / deck / slot / args, learn | `bindings bind` (`binding`), `bindings unbind` |
| Map | a cell (focus in Bindings) | view only; `bindings list` / `map` read it |
| Route | offset ms, the part × destination ticks, Clear all, Add destination, clear | `route offset`, `route set` / `toggle`, `route clear`, `route add_dest` |
| Action log | Clear, Save all, from / to bar, Save passage, Play passage, Stop | `log clear`, `log save`, `log arrangement` / `save` (`from_bar`, `to_bar`), `log play`, `log stop` |
| Vocoder | ON, INSERT, mute, PFL, FREEZE, XFADE, BYPASS, SYNC, waves | `vocoder` `on` / `insert` / `mute` / `pfl` / `freeze` / `xfade` / `bypass` / `sync`, `osc1_wave` … |
| Vocoder | the V / V2 / … tabs, + ADD, REMOVE | `vocoder` `action: select` (`instance`) / `add` / `remove` |
| Vocoder | ⇄ (swap) | `vocoder` `action: swap` |
| Vocoder | modulator / carrier menus | `vocoder modulator` / `carrier`, `cycle_modulator` / `cycle_carrier` |
| Vocoder | the knobs | `vocoder` (`bands`, `shift`, … or `knob` + `value`) |
| Vocoder | synth preset picker, name, Save | `vocoder preset`, `preset_save` (`name`), `preset_next` / `prev` / `delete` |
| Browser | source, kind, pack, search, load / audition buttons | `browse list` / `search` (`source`, `kind`, `pack`, `query`), `browse load` / `audition` / `stop` |
| HEOS | Rediscover, Refresh, Connect | `heos discover` / `refresh` / `connect` (`host`) |
| HEOS | zone ticks, volume, MUTE, group volume, Ungroup | `heos group` / `stream` (`zones`), `heos volume`, `heos mute`, `heos ungroup` |
| HEOS | format, port, Close server | `heos stream` (`format`), `heos.port` **(new, live while stopped)**, `heos stream_server_stop` |
| Aux | aim A / B / L / focus | per call: `aux macro … deck=X` takes the deck (the aim is the window's own) |
| Aux | a tile (click / right-click bars / shift-click stack), Fire stack | `aux macro=ID deck=X bars=N` (a stack is several calls); the same macro by `deck macro`, `perform {op: macro}` and the APC pad |
| Aux | PANIC | `deck panic` (ends the pan / shape / aux macros too), `aux cancel` |
| Aux | Hit 1-4 | `perform {op: stinger, deck, n}` |
| Aux | FX return, mute, the six sliders, Reset sliders, a send per strip | `aux return` / `mute` / `<slider id>` / `reset` / `strip`+`send`; the `auxReturn` / `auxMute` / `auxFx` / `auxSend` targets |
| perf overlay | (F3) | `perf` |

### Launch flags

| flag | runtime path |
|---|---|
| `SPEC_A SPEC_B` (genre:seed:structure, demo, a file) | `deck load` / `demo` / `file` |
| `--device` | `audio output`, `audio.output` |
| `--input DEVICE=LABEL`, `--no-inputs` | `audio input` (re-route a strip), `audio.inputs`; a new strip needs a restart |
| `--input-latency` | `input latency_ms`, `audio.input_latency_ms.LABEL` |
| `--history-beats` | `history beats`, `history.beats` (on / off: restart) |
| `--history-jog` | `history jog`, `history.jog` |
| `--cue-pairs`, `--sf2-dir`, `--plain`, `--no-start-muted` | restart (`audio.cue_pairs`, `audio.sf2_dir`, `engine.plain`, `audio.start_muted`) |
| `--controller` | `controller` (to / from `none`: restart) |
| `--play` | `deck play` |
| `--midi`, `--td3`, `--bindings`, `--map`, `--route`, `--log` | `panels.*` (and `route open`, `log open`) |
| `--pd705-steps` | `td3 pd705_steps`, `td3.pd705_steps` |
| `--loops-dir`, `--no-loop-library` | restart (`loops.dir`, `loops.library`) |
| `--control` | restart (`control.socket`) |
| `--heos`, `--heos-port` | `heos connect` / `discover`; restart for the mode (`heos.mode`); `heos.port` |
| `--screenshot`, `--size` | `screenshot`; restart for the size (`window.size`) |
| `--headless` | restart (a window or none is chosen at launch) |
| `--config`, `--save-config` | `config reload` / `save` |

## The MCP server

```sh
djctl app-mcp [--socket PATH]      # PATH: $DJAPP_SOCKET, else /tmp/djapp.sock
```

It's a stdio MCP server named `djapp` (so its tools show up as `mcp__djapp__djapp_*`) with one tool per
noun in the registry (`djapp_status`, `_state`, `_deck`, `_loopsets`, `_wheel`, `_describe`, … the
"Methods at a glance" table above lists them all, with the tool for each). The tool list is
generated from that registry, so it can't miss a method. Each tool call opens a
fresh connection, so djapp can restart under a live session. When djapp isn't running, a call
returns a tool error that says so. This is a different server from `djctl mcp`, which serves the
Swift app's `dj_*` catalog on `DJApp/control.sock`.

One tool is not a socket call: **`djapp_render`** renders an arrangement (inline, a file, or
the running app's own action log) offline in the MCP server's process, to a WAV and stems,
without touching what plays (`post` / `post_chain` finish the master, docs/POST.md).
**`djapp_post`**'s file actions (`process`, `measure`, `presets`) also run in the MCP server;
its `status` / `set` go to the socket. `djctl mcp` has the same file actions as **`dj_post`**. The socket also answers a windowless `djapp --headless` the same
way. Both are in docs/HEADLESS.md.

### As an MCP server over ssh

Install a release `djctl` next to the booth binary (`cargo build --release -p djctl`), then register it with your
MCP client as a stdio server reached over ssh, for example:

```json
"djapp": {
  "description": "djapp, the Rust DJ app on the booth Mac, through its control socket: status, decks/transport/tempo, mixer, FX, loops and captures, input history, audio output and input routing, MIDI clock / TD-3 / PD705, bindings. djapp_status first. It plays live in the booth: read state after acting, no sudden loud changes unless asked.",
  "command": "ssh",
  "args": ["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "<booth host>",
           "exec <path to>/djctl app-mcp --socket /tmp/djapp.sock"]
}
```

To use it straight from Claude Code on the Mac:
`claude mcp add djapp -- <path to>/djctl app-mcp`.
