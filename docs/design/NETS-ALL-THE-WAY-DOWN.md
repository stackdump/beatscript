# Nets all the way down

*Design, with a phase-1 proof on `feat/nets-down` (from `feat/btw-language`). Matt's direction: "push everything
down to the nets; it's maximum inspectability."*

**The claim.** A btw program is a performance. Today only its *notes* are nets. Its control lines (ramps, moves,
mixer and insert calls, `do` calls, the wheel, sounds, sections, `end`) run as timed calls *beside* the nets, in the
runtime (`crates/djlive/src/runtime.rs`) and the host's move scheduler (`crates/djhost/src/perform.rs`). The goal is that the
whole track, every control change included, is a net, so that **the marking is the complete state** and the engine's
job is only to *fire transitions and apply events*.

## 0. Summary

1. The engine already has most of what this needs: a deterministic tick executor (`WaveGraph`, one tick per sixteenth),
   transitions that carry a typed `control` binding (an `Other` control is *listed as fired and changes nothing else*), a
   seek (`fast_forward`), a marking mirror, a fixed `controls` ring, `p_invariants`, and slot groups with
   `activate-slot`. Nothing in this design needs a new kind of net.
2. A control line compiles to a **chain of transitions on the step clock**; a transition's **label** is the typed
   event(s) it emits (`set Master filter.sweep 0.43`, `mute A on`, `trick A filter on amount=0.9 bars=4`). The alphabet is
   small and closed (§3.1). The engine fires the net and applies the events.
3. A ramp is a chain with a **value table keyed by step** (exact integers for time, a precomputed value per step). A
   move is a **subnet with a restore transition that cannot fire before its own cut** (§3.4), which is the mute-forever
   fix by construction. Sections, `end` and `loop` are a clock that enables and disables deck nets (§3.7).
4. **CIDs:** every existing CID (program text, deck net, section net, generation) is unchanged. The control net is a
   separate document, and a new **performance CID** names decks + control (§4). One constraint found: the engine's model
   CID hashes only a control binding's `action` and `targetNet`, so the event label must live there, not in `macroParams`.
5. **Proof (phase 1, done):** `master: filter lp A -> B over N bars` and two moves (`filter_exit`, `echo_out`) compile to
   net transitions; the offline render applies their events; the spectral centroid rises 71 → 4600 Hz across an 8-bar
   ramp (flat ≈4900 Hz without the net), and a move's mute is undone on the phrase line, by the net (§8).
6. Measured on the real 64-bar `hand-dubstep-140-v2`: 43 control lines, 42 in the net; 2261 places, 2218 transitions,
   383 KB JSON; built in 7 ms, 1024 ticks run in 17 ms (release): about 16 µs a tick against a 107 ms tick.

## 1. What the engine already is (the facts the design rests on)

| fact | where | why it matters |
|---|---|---|
| A tick is a sixteenth (`PPQ = 4`); every net advances one tick per `advance`; enabled transitions fire in key order; a contested input place is decided by `deterministicRand(tick, place)` | `djrust/src/wave/net.rs` (`advance_with`, `resolve_conflicts`), `synth.rs:13` | the step clock is the engine's own; a control net must avoid contested places or the outcome is a seeded coin (§6) |
| A transition has a `midi` binding (a note) **or** a `control` binding `{action, targetNet, targetNote, macro, macroBars, macroParams}` | `djrust/src/pflow.rs:127-200` | the typed label has a home that already round-trips |
| `ControlAction`: `mute-track`, `unmute-track`, `toggle-track`, `*-note`, `activate-slot`, `stop-transport` are applied by the graph; any other action name is `Other`: **listed in `controls` (net, transition) and nothing else happens** | `net.rs:24-52`, `apply_control` | `Other` is exactly "emit an event"; `controls` is a fixed 1024-int array, no allocation on the audio path |
| Slot groups: `activate-slot` mutes the group's other nets and unmutes the target | `net.rs` (`ControlAction::Slot`) | sections as slots (§3.7) are already an engine feature |
| Seek: `fast_forward_jump` steps control nets to each action, skips silent chains and whole cycles, ends on a real `advance` so its events are the replay's | `net.rs:~890` | seek *is* a marking computation, in the engine, today |
| The marking is published (`marking_mirror`), fires and tick anchors go to overwrite rings the runner writes (`fire_ring`, `anchor_ring`) | `runner.rs` header, `wave/marking_mirror.rs`, `wave/event_ring.rs` | the debugger and the firing markers can read the engine, not the compile (docs/BTW.md says the event ring "would be a change in djrust"; `fire_ring` is in the runner's header: to confirm in phase 3) |
| The **model CID** hashes a control binding's `action` and `targetNet` only (`macro`, `macroBars`, `macroParams` are left out) | `pflow.rs:938-945` (`normalized_for_cid`) | beats' canonical form, pinned to the JS and Go engines: do not change it. The event label goes in those two fields |
| Insert knobs are smoothed per sample by a one-pole (`SMOOTH_SECONDS = 0.02`) | `fx/insert.rs:32` | a step target is already glided to; zipper noise is a tuning question, not a missing mechanism (§6) |
| The offline renderer plays an arrangement: 75 `DeckAction` ops plus 21 `LIVE_METHODS`; **no insert or mixer-insert op** | `session/arrangement.rs:51-72`, `host/deck/live_ops.rs:87`, `host/deck/offline.rs:403` | why the offline render could not hear a filter, and why the proof drives the engine's own API from the net's events |
| `Engine` exposes `set_insert_on`, `set_insert_param`, `set_deck_mute`, `trick`, `set_strip`, `set_master`, `set_crossfader` | `host/deck/inserts.rs`, `engine.rs`, `tricks.rs` | the whole phase-1 alphabet is applied by existing calls |

## 2. Inventory: everything that is not in a net today

Line numbers are on `feat/btw-language` at `dad7d72`.

| # | what | where it lives now | what is wrong with it |
|---|---|---|---|
| 1 | **Ramps** (`filter`, `delay`, `reverb`, `crush`, `resonance`, `level`, `low/mid/high`, `crossfader`, `do … a -> b over N bars`) | `Ramp` `compile.rs:227`, `Ramp::at`; sent each service pass by `Live::move_ramps` `runtime.rs:770` (a call when the value moved `RAMP_EPS = 0.002`) | host-side, wall-clock, not in any CID, not in the offline render, not in a marking |
| 2 | **Timed calls** (`every`, `at`, `on phrase`, `after`, `before`, `for N bars`) | `Timing` `compile.rs:196`; `Live::fire_bar` `runtime.rs:672`; undo bookkeeping `Item.undo_at` `runtime.rs:285` | the schedule is `Item` state in the runtime, not a marking; a seek cannot reconstruct which `for` is still held |
| 3 | **Moves** (`move filter_exit / echo_out / loop_roll / bass_swap / phrase_sync / vocoder_handoff`, `swap pairs with …`) | compile to `moves arm …` (`compile.rs:1325`); run by `perform::Moves` `perform.rs:371-640`, `Moves::service` `:535`, timed on the host's wall clock | two clocks (§3.4); the cut and its put-back are scheduled apart |
| 4 | **Move put-back** | `Control.restore` `compile.rs:1272`; `Live::fire` `runtime.rs:746-750`; `stop_with` `:509` | the mute-forever bug (§3.4) |
| 5 | **Mixer / insert / registry calls** | `Effect::Calls` → `LiveHost::call` `runtime.rs:58` → `control_call` | not in the render, not in a CID |
| 6 | **`do` calls and sugar** (vocoder, vocal, freeze, trick, loops, effects, aux, automix …) | `docall::compile_do` `docall.rs:127`, `derive_undo` `:325` | the registry's verbs are typed by the registry, not by the net |
| 7 | **The wheel** (`wheel follow / sync / step / interval`) | `do`-sugar → `wheel` noun (`djhost/src/control_registry/nouns.rs`) | a `Call` like any other, but with a host-side answer (a motor, hundredths of a second) |
| 8 | **Tempo** | `Compiled.tempo` / `DeckIr.tempo` baked into each deck's envelope (`compile.rs:123-145`); the runtime never sets tempo; F3 (tempo curves) will | a constant today; a curve needs a clock net |
| 9 | **Sections, `arrange`, chaining** | `Segment` `compile.rs:273`; the host loads the next section's nets a bar ahead: `Live::sync_nets` `runtime.rs:853` → `LiveHost::load_net` | structure is a host loop, not a net; seek must replay it |
| 10 | **`end`, `loop`, `stop`** | `Compiled.end/looped` ; `fire_bar` `runtime.rs:676-718` | an ended program is a field of the runtime, not a marking |
| 11 | **Evolve generations** | `Compiler::evolve` `compile.rs:712`, `evolve.rs`; each generation is already compiled as a *segment* | already net-shaped (precomputed nets swapped on the bar line); the rule itself is a function over hits |
| 12 | **Sounds** (`sound=`, `kit=`, `route=td3`) | `DeckIr::sounds`, `PendingSound` `runtime.rs:289`, `set_pending_sounds` `:642`, `td3_control` `compile.rs:843` | "after its net lands" is a host callback |
| 13 | **Deck transport** (`play`, `stop`, `mute`, `load`, a section not using a deck) | `Live::sync_nets` `runtime.rs:853-899` (`set_playing`) | host |
| 14 | **Hit lights / marks** | `Live::watch_fires` `runtime.rs:576`: *recomputed from the compiled hits and the grid beat*, not read from fired transitions | the "firing marker" is a model of the net, not the net |
| 15 | **Hot reload diff** | `diff` `runtime.rs:189`, `apply` `:796` (control items kept by key) | the same idea, held in runtime structs |
| 16 | **The offline render** | `btw_render::arrangement`; `djrust::host::deck::offline::render_arrangement` | no op for any of rows 1–7 |

## 3. Net encodings, one per kind

### 3.1 The event alphabet (small, typed, closed)

A transition's label is a list of events, in order, text form `a ; b`. The engine's job after firing is a `match` on a closed
enum, never a string parse (the text form is for the CID and for people; at load it is compiled to a table, §6).

| event | text | engine call (phase 1: exists) | covers |
|---|---|---|---|
| `Set{target, param, value}` | `set Master filter.sweep 0.431` | `set_insert_param`, `set_strip`, `set_volume`, `set_master`, `set_crossfader` | ramps, `level/low/mid/high/crossfader`, `filter/delay/reverb/crush/resonance` |
| `Insert{target, effect, on}` | `insert Master filter on` | `set_insert_on` | the "on" half of every insert line, `for N bars` undo |
| `Mute{deck, on}` | `mute A on` | `set_deck_mute` | `mute/unmute`, a move's cut and put-back |
| `Trick{deck, kind, on, amount?, bars?, beats?}` | `trick A filter on amount=0.9 bars=4` | `Engine::trick` | the filter-exit / echo-out primitives |
| *phase 2* `Loop{deck, beats \| halve \| exit}` | `loop A 4` | `loop_beats / loop_halve / exit_loop` | `loop_roll` |
| *phase 2* `Tempo{bpm}` | `tempo 128` | `set_tempo` | tempo setting and curves (F3) |
| *phase 2* `Deck{deck, play \| stop \| load spec \| seed}` | `deck A stop` | `play / stop / load / step_seed` | transport, `phrase_sync`, `end` |
| *phase 2* `Net{deck, section, load \| enable \| disable}` | `net A drop load` | `load_net` / slot | sections (§3.7) |
| *phase 2* `Sound{deck, role, option}` | `sound A bass synth:reese` | `set_sound` | `sound=`, `kit=` |
| *phase 2* `Call{noun, verb, params}` | `call vocal create text="…" bars=4` | `control_call` (registry) | `do …`, the wheel, `swap pairs`, anything not worth its own variant |

`Call` is the registry's verb as a typed label: the noun and verb are checked against the registry at compile time (they
already are), and the canonical text of the call is the label. It is the escape hatch that makes "everything is a net"
true without every registry verb needing an engine implementation: the engine's job for a `Call` is to hand it to its host's
registry. Web and Android implement the native variants and report the `Call`s they do not have.

### 3.2 A parameter as state

Three encodings, in order of how much of the state is *in the marking*:

| | encoding | state in the marking | cost | verdict |
|---|---|---|---|---|
| A | value on the transition label (phase 1) | where in each chain; the knob's value is the label of the last transition that fired | none | **phase 1–2**; value-at-bar-N needs the stream or the label table |
| B | a **level place** per parameter: its token count is the value in per-mille (`Set` becomes a transition that moves `|Δ|` tokens in or out through weighted arcs); the label stays as a checked redundancy | the knob's value, literally | the compiler tracks each parameter along the *static* timeline to know every `Δ`, so chains are unrolled per firing (no shared template for `every`) | **phase 5**: the "marking is the complete state" form |
| C | one-hot places over the finite alphabet of values the program ever writes; a write is a move from the known previous value; Σ = 1 is a P-invariant the engine's `p_invariants` can prove | the knob's value, exactly | one transition per write, one place per distinct value | alternative to B where weighted arcs are unwelcome |

The compile can do B and C because **the program is a finite static timeline** (every statement total, every count written out,
docs/BTW.md "Functions and structure"): the previous value of every parameter at every write is known before anything runs.
Where it is not (a live edit lands mid-chain) the compiler recompiles from the current tick (§6).

### 3.3 A ramp: a chain on the step clock, a table keyed by step

```text
clock   c0 -k0-> c1 -k1-> c2 ...                  silent; the transition before tick T also puts a token in whatever starts at T
ramp    r.s0 -t0-> r.s1 -t1-> ... -tK-> r.sK+1    K = 16 × bars; t_i is labelled  set P v_i
```

* `K + 1` transitions, one per sixteenth, each with its value: `v_i = shape(i / K)` computed **once, at compile time**, rounded
  to six places (what `Ramp::at` does today), and **a step equal to the one before it is not emitted**, so a held stretch of a
  sweep is silent. Time is token logic and exact integers; the only real numbers are table entries (and are part of the CID).
* Shapes (F4: `linear`, `exp`, `log`, `s-curve`, `step n`) are *other tables*: `shape::progress(shape, t)` is the one function
  both the old host ramp and this compile call, so there is a single definition of each curve. The net does not know the shape.
* The chain's first transition also carries the setup events (`insert Master filter on`).
* A chain is a **template reused by every firing**: `every 1 bars: … over 1 bars` is one chain tapped 16 times; two firings
  that overlap are two tokens in one chain, running concurrently.
* **Compact form** (phase 5, for very long ramps): one transition per end, labelled `ramp P from to over N shape`, the DSP
  interpolating between step targets (§6). The marking then shows "inside this ramp" and the net is O(1) instead of O(N).

### 3.4 A move: a subnet with a put-back that cannot fire before its cut

```text
r.s0 -t0(trick A filter on …)-> … -tL(mute A on ; trick A filter off)-> r.pend
r.pend + r.free  -arm->    r.muted                 r.muted + r.pulse -restore (mute A off)-> r.free
r.pend, ¬r.free  -absorb->                         r.pulse, ¬r.muted -drain->
r.pulse ← a token from the clock at every phrase line, and at the program's end
```

* `pulse` is a token the clock puts in on **every** phrase line (every 8 bars); `drain` throws it away when nothing is
  muted, so a pulse that comes at the same instant as the cut is not used (the put-back is the first phrase line
  **strictly after** the end, as today).
* `free + muted = 1` is a P-invariant of the subnet: *a strip is muted or free, never both, never neither*. The engine has
  `p_invariants` for proving it; the proof does **not** run it, and instead a test reads the sum off the marking at every tick.
* `restore` consumes `muted`, which only `arm` produces, which only a cut produces: **the put-back cannot precede its cut and
  cannot be lost while the net lives.** A second cut before the first put-back is absorbed (one put-back, not two).
* `echo_out` is the same subnet with the wash trick; `loop_roll` is a chain of `Loop` stages with a `release`; `bass_swap` is two
  `Set low` events and a swap on the phrase pulse; `phrase_sync` is one `Deck seed` / `Set crossfader` on the pulse;
  `vocoder_handoff` is one `Call`. All of them become *tables of events at offsets*, and `perform::Moves`'s wall-clock scheduler
  is left only for the moves a DJ arms by hand (the buttons).

**The mute-forever bug, by my reading of the code.** Two clocks schedule one thing. The put-back is computed by the runtime from
the *control's* fire bar: `end = fire + len` with `len` taken from `over` or **8** (`compile.rs:1264`), put back at the next phrase
line after that (`runtime.rs:746-750`). The cut is scheduled by `Moves` when the move *arms*, on the next bar line strictly after
the arm (`perform.rs:352`, `:418`), for `bars` which is `over` or the move's own default, **4** for `filter_exit` and `echo_out`
(`perform.rs:120-123`). So, for example, `at bar 6: move filter_exit on A over 1 bars`: the runtime puts the strip back on bar 8,
and the move cuts on bar 8 (armed on 6, lands 7, a bar long), in the same service pass, in whichever order the host runs the two:
if the put-back runs first the strip is muted and nothing is left to unmute it (it is muted until something else does it).
`btw stop` puts back at once while an armed move is still in flight, and an edit that drops the line does the same.
(This is a reading, not a reproduction; phase 2's equivalence test on a fake clock should pin it.) In the net there is one
clock, and the put-back is downstream of the cut in the net's own causality.

### 3.5 Mixer, insert, wheel and `do` calls

One transition per call, at the tick its `Timing` taps; the label is the event or the `Call`. `for N bars` is the same chain with the
undo events (`Control.undo`) at offset `16 N`. A call that has no native event is `Call{noun, verb, params}` and runs through the
registry as it does now, from a net transition instead of `Live::fire`.

### 3.6 Tempo and tempo curves (F3)

The control clock counts **beats** (sixteenths), so the bar clock never moves when the tempo does. `tempo A -> B over N bars` is a
chain whose steps emit `Tempo{bpm}` from F3's table (its `TempoMap` integrates seconds in closed form; nets need only `bpm` at each
step). The engine's tick rate follows `set_tempo`. Seconds, `check`'s length and the offline render's frame of a beat all come from
the same `TempoMap`.

### 3.7 Sections, `end`, `loop`: a control net that enables and disables deck nets

The clock **is** the song position. Section boundaries are taps (a section is `start`, `bars`), and each tap fires the structure
transition of its section. Two ways to switch a deck's nets, both with one engine mechanism each:

* **Slots (preferred when it fits):** every section's net for a deck is in the deck's project as one net of a slot group per role;
  the structure transition fires the engine's own `activate-slot`. Nothing is loaded on a bar line, nothing is swapped from the host,
  and a seek is `fast_forward`. Cost: every section's rings are resident (about 7 ms and 3 MB a bar, docs/BTW.md), so a budget (say
  256 resident bars a deck) decides which sections are slots.
* **Load (the rest, and the 64-bar chain):** `Net{deck, section, load}` a bar ahead, then enable on the line: today's
  `sync_nets`, driven by the net instead of by the host's own bar counter.

`end` is a transition on the end tap that emits `Deck{stop}` for each deck and runs the put-back of everything held (the
`pulse` the moves already listen for), then marks a place `ended`: *ended is a marking*. `loop` makes the clock a ring and re-arms the
taps; a `Now` line fires again each lap, as it does now. `stop` is an event from outside (§6).

### 3.8 LFO (F4)

`lfo wave period depth= center= phase= over N` is a **cyclic subnet**: a ring of `period × 16` places, one token, a transition per
step labelled `set P (center + depth · wave(2π (k / n + phase)))`. A ring with `over N` is cut by the clock (a `stop` tap) or by an
**off place**: every ring transition has an inhibitor arc from `off`, so marking `off` freezes the token where it is and
clearing it resumes. The same inhibitor-gate enables or disables any subnet, and is what a section end uses for "the lfo runs until
the section ends".

### 3.9 Evolve generations

Already net-shaped: `Compiler::evolve` compiles each generation as a segment of the timeline, so each is a slot or a loaded net in §3.7
and the section's structure transitions walk them. The **rule** is not turned into transitions: `rotate`, `transpose`, `thin` act on
realized hits (data), and a net has no way to compute on them without encoding hits in places, for no gain. Keep the orbit analysis
and `generation_cid(n)`; precompute the generations the section plays (at most its length in generations); record each generation's CID
in the control net's `lineage` so the marking says which generation is playing.

### 3.10 Sounds and transport

`Sound` and `Deck` events on the tap where the net lands: today's `PendingSound` list becomes a transition after the section's.

## 4. CIDs

**Today:** the program CID is of the canonical *text* (`compile.rs:520`); deck, section and generation CIDs are of the *notes*
(`DeckIr::net_cid`). **Nothing covers a control line**, so two programs with the same notes and different filter sweeps have the same
net CIDs (the proof's `deck_net_cids_do_not_move…` test shows it), and any cache keyed by net CIDs would collide.

**New:** `performance_doc` = `{format, type: "djrust.btw.performance", tempo, decks: {deck or section@bar:deck → net CID}, control: <control net CID>, beside: [control keys not yet in the net]}`; `performance_cid = share_cid` of it
(`djlive::nets::performance_cid`, in the proof). The control net's own CID is `share_cid` of its project; its **engine model CID** is `Project::model_cid`, and both change when a single ramp value changes (tested), because the label is in `action` / `targetNet` (§1).

* **Programs with no control lines:** no control net, no `control` key; every existing CID is untouched. The performance CID is a *new* name over the same deck CIDs.
* **No migration for shares, forks or imports:** they are of the text. `forkedFrom`, `import` pins, `.btw` headers, share links are not touched.
* **Migration where something is keyed by net CIDs alone:** a render cache or CDN key built from deck net CIDs must add the performance CID (a swept and an unswept program of the same notes collide today). Check the render farm (`beats-builder`) and `djctl btw render`.
* **Phase 4, one program project:** put decks and the control net in one `Project` (`nets: {A, B, ctl}`) so the *model CID of the program's net* is the performance's. That makes the deck nets' envelope CIDs parts of it, not replaced by it.
* **Intensional vs extensional (open question 3):** `repeat 16 { master: filter lp 300 -> 2800 over 1 bars }` and `every 1 bars: …` compile to different nets (16 chains vs one tapped 16 times) with the same events. Net CID is intensional. If "two texts, same sound" must give one CID (as `song_nets` does for notes), add a **timeline CID**: the hash of the `(tick, event)` stream. Recommend both.
* **The horizon** is part of a finite net: a program with a timeline has it (`song_bars` / `end`); a program that loops forever uses its period (lcm of its rings) or its 64-bar stretch. The proof takes it as an argument.

## 5. What inspectability gets

* **Seek = a marking computation.** `ControlNet::marking_at(tick)` is the whole control state at a sixteenth; `events(Some(t))` is exactly the prefix of the full stream (tested). In the engine `fast_forward` already does this for the notes.
* **The debugger reads places.** The clock token's place is the playhead; a chain's token is how far a ramp is; `muted/free` is the strip's put-back; `ended` is whether it ended. `ControlNet.lines` maps each line's key to its transitions, so a place is a line in the editor.
* **Firing markers derive from fired transitions.** `Live::watch_fires` (row 14) recomputes hits from the compiled hits and the beat; with `controls` and the fire ring a marker is *a transition that fired*, so a control line's glow, a ramp's progress bar and a hit's lamp are one mechanism.
* **The offline render hears everything** (proved: the sweep and the move are in the WAV) and a render is a function of the performance CID.
* **Web and Android run one engine with no runtime glue.** The glue today is `Live` + `LiveHost` + `perform::Moves` in `djlive`/`djhost`. With phase 3 the engine's `WaveGraph` (`no_std`, builds for wasm32) fires the control net and applies the events itself; the host provides only the `Call` fallback.
* **Static checks become net checks:** the invariant `free + muted = 1`, "no place has two consumers" (no random choice), "every transition is on a path from the clock" are `p_invariants` / incidence questions, not code review (none is run in the proof beyond the marking check above).

## 6. Costs and risks

| | finding | answer |
|---|---|---|
| **Net size** | real 64-bar track: 2261 places, 2218 transitions, 4477 arcs, 383 KB JSON, against 1.1 MB for its two decks' nets. 1025 of the places are the clock chain; the rest are chains (a ramp is `16 × bars` transitions) | chain per **stretch** of ≤ 64 bars (as decks are); compact ramp form (§3.3) for long ramps; a hard cap on transitions per stretch with a diagnostic |
| **Audio-thread budget** | the engine's `advance` is O(transitions) per tick; measured 16 µs a tick (release, includes building the graph) on that net, against a 107 ms tick at 140 BPM. Events go out through the fixed `controls` array (512 pairs a tick) | the label text is compiled at load to a table indexed by transition (`ctl_event[t]`), so applying is an index and a `match`, off the allocation path; nothing is parsed or built on the audio thread |
| **Continuous vs discrete (zipper noise)** | a step target every 1/16 bar (107 ms at 140) is coarser than the ~60 Hz the host ramp sends today; the proof sends step targets only and the engine's own 20 ms glide fills between them | *untested by ear*. Fixes, cheapest first: a `glide=<ticks>` on a `Set` so the DSP interpolates to the next target over the step (no change to the net); a finer control clock (×4, size ×4); the compact ramp form. Needs a listening test on a resonant sweep |
| **Live edits on the bar line** | today a changed program keeps running lines by key (`apply` `runtime.rs:796`), loads a changed net on the next bar. A net swap must carry the marking | diff control subnets by line key as now: an unchanged subnet keeps its marking (a ramp in flight goes on); a new one starts; a **removed** one runs a `retire` transition (its put-back and undo) before it goes, so removing a move's line cannot strand a mute. The clock position is carried by tick |
| **64-bar chaining** | a deck net is at most 64 bars; a program is chained from stretches. The control clock is a chain of 1024 ticks | the same stretching; a ramp that crosses a stretch is two chains whose second starts from an initial token (the timeline is static, so the compiler cuts it) |
| **Determinism** | contested input places are decided by a seeded coin | the compile builds none: every shared input place has consumers that exclude each other by an inhibitor or a complement place (`free`/`muted`). Add a check |
| **Zero-length / sub-sixteenth** | F1 (sub-bar timing) allows `beat 1/3`; a sixteenth clock cannot | snap with the same warning patterns give, or a finer control clock; open question |
| **Event ordering within a tick** | events of one tick come in transition key order, which is program order | pinned by a test; same as the order `fire_bar` runs items |

## 7. Phased plan, with tests

| phase | what | tests |
|---|---|---|
| **1 (done, this branch)** | ramps and two moves into a net; `djlive::nets`; `djhost::btw_render::render` applies the events to the offline engine; performance CID | §8 |
| **2** | the rest of `Effect::Calls` (`Call` event: mixer, `do`, wheel, `swap pairs`), `for N bars`, `on phrase / after / before`, `loop_roll`, `bass_swap`, `phrase_sync`, `vocoder_handoff`, sounds; **the runtime drives from the net** behind a switch; `djctl btw render --controls` | **dual-run equivalence**: the old runtime and the net on the same fake clock must produce the same call stream for every program in `tests/` and the three track fixtures (differences listed and explained, not hidden); the mute-forever reproduction; hot reload by key (a ramp goes on through an edit; a removed move is put back) |
| **3** | the engine applies events: `CompiledNet` gets a `ctl_event` table, `StudioShell`-style dispatch for the native variants (a PR in the public `djrust`); marks read the fire ring; the debugger reads the marking mirror; seek by `fast_forward` | a golden parity test of the net on the engine vs the host applying events; a seek to bar N equals playing to bar N (events and marking); wasm32 build of the control net |
| **4** | sections as slots / `Net` events, `end`, `loop`, `stop` as nets; evolve generations as slots; the program project with one model CID; the timeline CID | a song's structure net equals today's `sync_nets` landings on a fake clock; `end` puts back everything held; loop laps |
| **5** | parameters as level places (B/C), LFO and shapes from F4, tempo curves from F3, the compact ramp, `glide` | P-invariants for every parameter; state-at-bar-N from the marking alone equals the parameter state after playing to N |

## 8. Phase 1: the proof

**Code** (branch `feat/nets-down`, from `origin/feat/btw-language`):

* `crates/djlive/src/nets.rs`: `Event` (the phase-1 alphabet with its text form), `lower` (a `Control` to events at offsets), `control_net(compiled, bars) → ControlNet` (the project JSON, `lines`, `residual`), `ControlNet::{events, marking_at, runner, cid, model_cid, size}`, `performance_doc`, `performance_cid`. It builds an ordinary net project and runs it on the engine's `WaveGraph`; it adds no executor.
* `crates/djhost/src/btw_render.rs`: `apply_event` (an `Event` to the `Engine`'s own calls) and `render(compiled, sample_rate, controls)`: the arrangement's moves and the net's events on one timeline, each on the frame of its beat.
* `crates/djlive/examples/control_net.rs`: size, CIDs, events and what stays beside the net, for a `.btw` file.
* Left alone: `runtime.rs`, `compile.rs` (a `lib.rs` line and nothing else in `djlive`), the registry, `perform.rs`, `djctl` (`djctl btw render` does not use the net yet: phase 2).

**Results:**

| test | what it proves | result |
|---|---|---|
| `djhost/tests/nets_render.rs::a_filter_sweep_written_in_btw_is_audible_in_the_offline_render` | `master: filter lp 250 -> 12k over 8 bars` over hats + bass + kick: the spectral centroid at bars 0, 2, 4, 6, 7.5 | **71, 108, 373, 2043, 4600 Hz** with the net; 4903, 4936, 4867, 4821, 4939 without it (the control render). Rises at every step; first window < half the open render; last window ≥ 60% of it |
| `…::a_move_cuts_the_strip_and_the_phrase_line_puts_it_back_in_the_render` | `at bar 2: move filter_exit on A over 2 bars`: RMS bar 1 / bar 5 / bar 9 | **0.1363 / 0.0000 / 0.1532**: cut on bar 4, put back on the phrase line, by the net |
| `djlive/tests/nets.rs` (13) | ramp = 65 table steps ending on its bar line, equal to `Ramp::at`; tapped to its bar (first value on the line, not a tick after); overlapping `every`; typed labels round-trip; move cut / put-back on the next phrase line, **strictly after** an end that is itself a phrase line; two moves, one put-back; `free + muted = 1` at every tick; marking is the state (events to tick t are the stream's prefix); residual lines named; CID covers every value; no control lines → no net; deck net CIDs unchanged while the performance CID differs | 13 / 13 |
| full `cargo test -p djlive` | nothing regressed (161 tests) | pass |

**What is in the net and what is not** on the real track `docs/btw/hand-dubstep-140-v2.btw`: 42 of its 43 control lines; the one
beside the net is `before drop 4 bars: move loop_roll on A over 4 bars` (needs the `Loop` event, phase 2).

**Semantic differences from the host path, on purpose, to decide (open questions 1–2):** a move lands on the bar line it is armed on
(live: the next line after); `over` missing means the move's own default of 4 bars for the cut (live: the cut uses 4, the put-back
computation 8), and the net's put-back is the phrase pulse whatever the length.

## 9. Conflicts with the other branches

| branch | state | conflict |
|---|---|---|
| `feat/btw-extensions` (F1–F14, 10 commits: base, F6 meter, F7 chance) | `Timing`, `Effect`, `Control`, `Ramp` are extended there ("add enum variants at the END") | **F1 sub-bar timing**: `Timing` gets beat and rational positions; `nets::triggers` takes a position × 16 and must say what a position off the sixteenth does (§6). **F3 tempo / F4 shapes+lfo** must hand *tables* to `nets::lower` (one `shape::progress`, one `TempoMap`), not call a host; today `lower` calls `Ramp::at` (linear). **F8 `mix {}`** (put back at a section's end) is the same restore idea with static values: set events on the section-end tap, no pending token; `duck B under A.kick` is events at the hits' ticks. **F12 `assert length`** reads `TempoMap`. **F13/F14** add to `live_store` and the saved object: the performance CID belongs there. *Files:* `lib.rs` (both add `pub mod`: trivial), **`djhost/src/btw_render.rs`** (F3/F12 edit `arrangement`; mine only adds below it and a header: expect a textual conflict, keep both), `compile.rs` (untouched by me). A new `Effect` variant is a wildcard in `lower`: it lands in `residual`, never a silent drop |
| `feat/btw-web` (`btwapi::run`: `Live` behind a command-queue host `load_net / play / sound / call`) | wasm module `djweb` over `djlive` | phase 2 makes covered lines stop calling `host.call`; their command queue gains one command kind (`Event`). Phase 3 moves application into the engine, which is what they want (no runtime glue). `nets.rs` uses `djengine::wave::WaveGraph` (`no_std` + alloc); **I did not build `djlive` for wasm32**, so that is to be checked |
| `design/btw-debugger` | **no commits ahead of `feat/btw-language`; its worktree is clean at the base** | I could not read a plan. Designed to the brief: the debugger reads the marking (`marking_at`, the engine's marking mirror) and fired transitions (`controls`). Conflict is likely to be in `runtime::Marks` (`marks()`, `ControlMark`), which phase 3 would feed from fired transitions |
| `feat/btw-android` | no commits ahead of `feat/btw-language` | benefits from phase 3; no conflict |
| `docs/BTW.md` | says the offline render has no control lines; "an event ring would be a change in djrust" | update with phase 2/3 (not touched here: others edit it) |

## 10. Open questions for Matt

1. **Where does a move land?** `at bar 16: move filter_exit on A over 4 bars` starts on bar 16 in the net. Live today it starts on bar 17 (armed on 16, lands on the next line strictly after). Which one should the language mean? Every existing track with a `move` changes by a bar if the net's reading is chosen.
2. **`over` missing on a move:** the cut uses 4 bars, the put-back computation 8. One default, which?
3. **CID: intensional, extensional, or both?** Net CID (this proof) distinguishes `repeat` from `every`; a timeline CID (hash of the event stream) does not. Both? Also: a program that loops forever has no horizon: its period, or a stretch?
4. **Where does event application live?** Public `djrust` (a typed `ctl_event` table on `CompiledNet` and native dispatch: web and Android get it free; a PR to a public repo) or `djhost` (private, quicker; web and Android reimplement). Recommended: djrust.
5. **The label in `action` / `targetNet`.** It is covered by the model CID only because beats' canonical form hashes those two fields. Fine to rely on, or would you change the model CID to include `macroParams` (breaks parity with the JS/Go CIDs of nets that already carry `fire-macro` params)? Recommended: keep.
6. **Step size.** One value per sixteenth: is it smooth enough by ear on a resonant sweep, or do you want a `glide` / finer clock first? I cannot listen; the spectral test says the filter is moving, not that it is clean.
7. **Sections as slots or loads** (memory vs simplicity), and the resident-bar budget.
8. **Hand-armed moves** (the DJ's buttons) keep `perform::Moves`: only program lines go through the net. OK?
9. Should `djctl btw render` use the net by default once phase 2 has the equivalence test, with `--no-controls` for the old output?
