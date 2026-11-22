# btw debugger: breakpoints, hold, time travel, and prompt-in-context

*Design only. Nothing here is implemented.* Branch `design/btw-debugger`, cut from `origin/feat/btw-language`
at `dad7d72`. Written 2026-11-17 for Matt.

**The ask.** A user of btw (djapp on the desktop, btw.bitwrap.io on the web, djandroid on the phone) can
pause playback at a chosen place, see exactly where they are, and say "add a transition effect here",
"make this bar halftime", "drop the hats for 2 bars". A model gets that exact context and answers with a
**small, checked edit** to the program text. The user previews it, A/B's it against the original, accepts
or rejects it, and an accepted edit is a **fork**.

## 0. The design in one page

The whole thing rests on one property btw already has and a debugger usually has to fight for: **a btw
program is a deterministic function of its text and a bar number.** So a debugger is not a recorder. It is
a *transport* (play, hold, pause, step, seek) over a *pure function* `frame_at(program, bar)`.

Eight decisions, each argued in the section named:

| # | Decision | § |
|---|---|---|
| 1 | **`frame_at(compiled, bar) -> Frame`** is a pure function in `djlive`. The runtime's own timing and ramp logic is refactored to call the *same* functions, so "seek equals played" holds by construction and the tests are a backstop. | 2 |
| 2 | **Breakpoints live outside the program** (session state, anchored structurally: `drop+8`, a control line's key, a generation), never in the text or the CID. The debugger verbs are in a noun that **programs cannot call**, so a program can never pause itself. | 3 |
| 3 | **Pausing is a transport mode over a virtual program clock**: `hold` (loop a window of the *program*, default the pending bar, using derived slice nets), `pause` (silent), `soft` (v2). Resume always lands on a bar line. | 4 |
| 4 | **Seek = hold at `frame_at(N)`**, rotating each net to the right alternation phase. Exact for the score layer; verbs with external state are classed `Fold`, `Replay` or `Opaque`, and the frame says which it could not reproduce. | 2, 4 |
| 5 | **Inspect = `Frame` + `Provenance`**: the same structure answers "where am I", "what plays next bar", "why is this sounding" and "why is this silent". Net markings come from the compiled IR (everywhere) and are cross-checked against the engine's marking mirror (desktop). | 5 |
| 6 | **The model is outside the pipeline.** A `Proposal` is a list of **statement-anchored ops**. It is applied, `check`ed, **footprinted** (which *bars* change sounding, not which bytes), previewed, and committed as a fork. The same pipeline serves Claude over MCP (v1) and the hosted compose route (v1.1). | 6 |
| 7 | **Transitions are a standard library of `def`s**, `import`ed **by CID**: `std/transitions`. `import` already brings lets and defs, pinned in the canonical text. No language change for v1. | 7 |
| 8 | **Surfaces share one core.** `djlive` owns `Frame`, `Debugger`, `Proposal`; `djapp`, `djweb` (wasm) and `djmobile` (Android) are thin hosts. | 8, 9 |

**Recommended v1** (details in §10): headless-first time travel (`frame_at`, `seek`, inspect), desktop
`hold`/`step`/`resume` with bar, section, line, generation and score-condition breakpoints, and the
**MCP-driven** edit loop (Claude calls `propose`, the app validates, previews by hold-loop, A/B's, accepts
as a fork). No engine change, no grammar change. Web and Android follow in v1.1 on the same core.

### What I read, and how sure I am

| Source | Status |
|---|---|
| `docs/BTW.md` on `origin/feat/btw-language` (grammar, canonical form, evolve, marks) | read in full |
| `crates/djlive/src/runtime.rs` (`Live`, `service`, `watch_fires`, `marks`, `status`), `compile.rs` (`Compiled`, `Control`, `Timing`, `Generation`), `expand.rs` (call time words), `diag.rs` (`Span`) | read; claims below about them are verified |
| `crates/djhost/src/btw_render.rs`, `docs/HEADLESS.md` (offline render) | read; the "control lines are not rendered offline" limit is BTW.md's own and is the biggest dependency (§10, R1) |
| The compose route: `btwsite/DESIGN.md`, `internal/btwsite/compose/{service,handler}.go` in the **unpushed** worktree `beats-bitwrap-io/btw-site` (`origin/feat/btw-site` is not on the remote; only `feat/btw-link` is) | read; the route may still move |
| Firing markers / follow: `crates/djapp/src/btw_follow.rs`, BTW.md "Seeing it play" | read |
| djandroid `feat/btw` (`rust/src/btw/`, `ComposeClient.kt`, `BtwSheets.kt`) | skimmed: stubs and signatures only |
| The engine's marking mirror (`readMarking`, `markingLayout` in `djrust/src/host/deck/views.rs`) | **seen, not exercised**: I have not checked what it reports for a one-bar slice net. Spike S1 in §10 |
| Engine behaviour for `load_prepared` phase, pause-with-tails, scheduled sub-bar stops | **assumed**; spikes S2, S3 in §10 |

Naming: the user's brief says `djwasm`. In this tree the wasm is `crates/djweb` (on `feat/btw-web`, over
`crates/btwapi`), and `djwasm` is the public engine facade `djrust::wasm` that `djweb` re-exports. This
document says `djweb` for the language side.

---

## 1. Ground truth: what exists, and what that forces

### 1.1 What the runtime does today

`Live` (`crates/djlive/src/runtime.rs`) is a hot-reload runtime driven by a host clock:

- `Live::service(host)` reads `host.grid()` (deck, beat, bpm), advances `self.line` by
  `lines_crossed(prev, beat)`, calls `fire_bar` for each bar crossed, applies a staged program in the first
  three quarters of a bar (`APPLY_BY = 0.75`), loads the next section's nets a bar ahead (`sync_nets`),
  moves ramps, and derives *firings* by comparing the position between passes (`watch_fires`).
- `line` is the **program bar**; an arranged song counts from `origin`, and `section_at(line - origin)`
  gives the section. Frames in this design are addressed by `rel = line - origin` (song-relative), so a
  frame does not depend on when the program happened to start.
- `Running.items` holds control-line state: `fired`, a `RampRun` (start bar, last value, done), `undo_at`.
  **That state is history-dependent**: `Timing::Now` is "once, on the first bar line after the statement is
  added or changed", and hot reload keeps a running ramp and does not re-run a `now` line.
- Everything the UI lights (`Marks`: `active_lines`, `header`, `fires`, `controls`, `generation`) is derived
  from `Compiled` plus the position. A hit carries `spans` (token first, call sites last) and `calls`.
- `evolve`: `Compiled::generation_cid(scope, n)` is computed from generation 0 and the rule
  (`evolve::nth`), "no chain stored"; `generation_at(rel)` maps a bar to a generation.
- A deck's net is compiled to per-part `cycles` (one hit list per bar of the part's alternation); sections
  are `Segment`s with their own nets, loaded a bar ahead and landing on the bar line.

### 1.2 What that forces

| Fact | Consequence for the debugger |
|---|---|
| The program bar is *derived from the grid clock* (`line += lines_crossed`), not owned by `Live`. | Pause/hold/seek need an indirection: a **virtual program clock** `line = grid_line - shift` (§4.2). Without it a hold would have to stop the engine. |
| Control state is history-dependent (`Now` lines, hot-reloaded ramps). | A *seek* is defined as **fresh-play semantics**: "the state as if this text had played from bar 0". An edit that was hot-reloaded mid-run can differ in `now` lines and ramp phase. Previews therefore **replay** the old program to bar N on a virtual clock, then apply the edit (§6.5). The inspector shows a `fidelity` field instead of pretending. |
| A net loops and keeps its own alternation phase; a freshly loaded net "starts at the same place" as the old one. | After any seek or hold, `shift != 0`, so a net with `<a b c>` must be **rotated** to the right phase. Rotation is a compile-time, content-addressed derivation (`DeckIr::rotated`), no engine change (§4.3). Spike S2 verifies the engine side. |
| `btw render` / `btw_render.rs` build an arrangement from sections only; **control lines are not in the offline render**. | An offline preview of "add a riser" would be silent where the riser is a filter ramp. The **live hold-loop preview does not have this problem** (it runs the engine and the runtime). So v1 previews by hold-loop; offline preview is gated on prerequisite P0a. |
| `import` brings **lets and defs only**, pinned by CID in the canonical text. | A transitions library is expressible *today* as pinned defs (§7). |
| A call's time word sets where the def's body starts (`expand.rs`, `base`): `at bar N:`, `after X:`, `before X N bars:`. | `before drop 4 bars: riser(4)` already composes; no new timing syntax needed for a library. |
| The registry already has the vocabulary of transitions: `trick` kinds `stutter, pump, delay, wash, verb_freeze, tape_stop, spin_back, half_time, double_time, filter, riser`; `deck role_disable`; `move` kinds `filter_exit, echo_out, loop_roll, bass_swap, phrase_sync, vocoder_handoff`. | The stdlib is thin, named, checked wrappers over verbs that exist. |
| The compose route today is **whole-program**: `POST /api/compose` with `base_cid` hands the model the base program and gets a whole program back; the base must be a **public, CC0, unrestricted** program in the store or the examples. | Debug-time edits need an **edit mode** (selection, window, ops back) and must keep the same limits. Private or restricted programs never go to the hosted model (§6.8). |

---

## 2. The core: `Position`, `Frame`, `frame_at`

New module `crates/djlive/src/frame.rs`. Pure, no I/O, builds for wasm32 like the rest of `djlive`.

```rust
/// A place in the song: song-relative bar (0-based) and a sixteenth within it.
pub struct Pos { pub bar: i64, pub step: u8 }          // step 0..15

/// Everything that determines what the program sounds like at the *start* of `bar`.
pub struct Frame {
    pub rel: i64,                                      // song-relative bar
    pub section: Option<SectionPos>,                   // name, bar in section, section bars
    pub phrase: PhrasePos,                             // phrase number, bar in phrase (8-bar phrases)
    pub generation: Vec<(String, u32)>,                // per evolve scope
    pub decks: BTreeMap<Deck, DeckFrame>,              // net, per-part cycle phase, this bar's hits
    pub controls: Vec<ControlState>,                   // every control line, in program order
    pub params: BTreeMap<ParamKey, f64>,               // mixer / inserts values the program has set
    pub ended: bool,
    pub fidelity: Fidelity,                            // Exact | Replayed(lines) | Approximate(lines)
}
pub struct DeckFrame {
    pub net_cid: String, pub segment: usize,
    pub parts: Vec<PartFrame>,                         // role, cycle i of n, hits [step, note, vel, sid]
}
pub enum ControlPhase { Waiting { fires_at: Option<i64> }, Ran { at: i64 },
                        Ramping { from: f64, to: f64, start: i64, bars: u32, progress: f64, value: f64 },
                        Held { until: i64 }, Done }
pub fn frame_at(c: &Compiled, rel: i64) -> Frame;
impl Frame { pub fn digest(&self) -> [u8; 32]; }       // sha-256 of canonical JSON
```

### 2.1 Why this is cheap and exact

- **Nets and hits:** `segments.rposition(|s| s.start <= rel)` then `cycle = (rel - start) mod n` per part
  (what `watch_fires` already does inline).
- **Controls:** for each `Control`, `Timing` gives fire bars in closed form: `Every(n)` = multiples of n
  (never 0), `EveryFrom(n, f)`, `At(b)`, `Now` (the clean-run bar), `NextMult(n)`. A ramp's value at
  `(bar + frac)` is `a + (b - a) * progress`. `hold` (`for N bars`) gives the put-back bar. Moves'
  `restore` gives the unmute bar.
- **Evolve:** `generation_at(rel)` plus `evolve::nth` for the realised parts of generation *g* from
  generation 0. This is the "generation-n-from-gen0" property: no chain is replayed, and a generation
  far down an orbit costs the orbit, not *n*.
- **Cost:** O(segments + controls + orbit) per frame. A scan of a 16,384-bar program (the language's cap)
  is ~10^6 simple operations.

### 2.2 One definition, two callers

The risk with a second implementation of timing is that it drifts from the runtime. So P0b **moves the
logic out of `runtime.rs` into `timeline.rs`** (`fires_on(timing, rel)`, `ramp_value(ramp, pos)`,
`held_until(...)`) and has `Live::fire_bar` / `move_ramps` *call it*. `frame_at` calls it too. The
"seek equals played" test (§10) then guards the remaining glue, and a bug found there is a bug in the
runtime as well. This is the "structure over checks" move.

### 2.3 Fidelity: what a seek can and cannot reproduce

Every control line compiles to registry calls. Each registry action gets a **seek class**, added to
`control_registry::Action` (default `Opaque`, so an unclassified verb is conservative):

| Class | Meaning | Examples | At a seek |
|---|---|---|---|
| `Fold` | A set of parameters; the state is the last write in program order. | `mixer`, `inserts` (filter/delay/reverb/crush/resonance), `crossfader`, ramps of those | folded into `Frame.params`; **exact** |
| `Replay` | Idempotent and re-executable. | `deck load SPEC` (deterministic by spec), `play`, `stop`, `mute`, `sync`, `role_disable`, `trick on/off`, `freeze on/off` | re-executed in order from the last relevant line; **exact if the host re-executes** (`Replayed`) |
| `Opaque` | Depends on state outside the program. | `autodj`/`swap pairs`, `queue`, `loops`, `vocoder`, `vocal`, `wheel`, `td3`, `record`, `mashup`, `moves arm` | **not re-executed**; listed in `Frame.fidelity = Approximate(lines)` |

`Frame.fidelity` is the honest answer to "can I trust this seek?". A strict mode that *refuses* `Opaque`
verbs in a program being debugged is a possible later guard (open question 7).

### 2.4 Debug-installed nets: slice and rotate

Two derivations on `DeckIr`, both pure and content-addressed (so their CIDs are stable and cacheable):

- `slice(rel, len)`: the cycles for bars `[rel, rel+len)` as a `len`-bar looping net (with the section's
  sounds). A one-bar hold is a one-bar net that loops by itself: loaded once, no reload per bar.
- `rotated(k)`: cycle *k* becomes cycle 0, so a full net installed at a shifted clock plays the right
  alternation phase.

A debug-installed net has its own net CID (tagged in the inspector as `rot`/`slice` of the program's net).
**The program CID never changes** and `frame.digest()` is computed at the program level, so debug nets
never leak into CIDs, saves or links.

---

## 3. Breakpoints

### 3.1 Kinds

A breakpoint is a typed record. Every kind resolves to a **position** (`Pos`) or a **predicate over
frames**, and that resolution is a **total function** with an explicit "orphan" result.

| Kind | You write | Resolves to | How it is found |
|---|---|---|---|
| **bar** | `at bar 24` | `Pos{24,0}` | direct |
| **section** | `at drop`, `at drop+8`, `at drop end`, `before drop 2 bars` | the section's start/end plus an offset | `Compiled.segments` by name; orphan if the section is gone |
| **line / token** | gutter click on a part line or a token, or `on token 31:41` | "the next bar in which this token fires" | the **site** (below) matched against each frame's hits |
| **control line** | gutter click on a control line, or `on control "master: filter hp 100 -> 5000 over 8 bars"` | the next bar this line fires | `Control.key` (the canonical text of the line, which is exactly how `Live` already identifies a line across evaluations) |
| **evolve generation** | `on gen 3`, `on gen drop 3` | the start bar of that generation | `Compiled.lineage` (scope, gen, start) |
| **condition** | `when deck A level > 0.8`, `when gen >= 3 and section == drop` | the first bar whose frame satisfies it | **scanned** (score conditions) or **sampled live** (live conditions), below |

A **token site** is `(scope, deck, role, token ordinal within the part's pattern text)`, not a byte offset,
so it survives edits elsewhere. Matching uses the `sid`/`spans` a `Fire` already carries. After each eval
the breakpoint is re-resolved; an unmatched site is `Orphan(reason)`, shown ghosted in the gutter, never
fires, and is reported by `debug breaks`. (Dropping silently is worse; guessing a nearby token is worse.)

Options on any breakpoint, taken from the debugger tradition (gdb, DAP):

- `hits=N` (ignore count / "hit condition"), `once` (temporary, `tbreak`), `enabled`;
- `action`: **`hold`** (default), `pause`, **`log`** (record the frame and keep playing: a logpoint, handy
  for "show me every bar the drop's filter is below 0.3"), **`ask`** (stop and open the Ask box prefilled
  with this position: "break and prompt");
- `label`, free text.

### 3.2 Conditions are typed so they can be scanned

```ebnf
cond     = or ;           or  = and { "or" and } ;     and = not { "and" not } ;     not = [ "not" ] cmp ;
cmp      = operand ( "<" | "<=" | ">" | ">=" | "==" | "!=" ) operand ;
operand  = number | word
         | "deck" DECK ( "level"|"low"|"mid"|"high"|"mute"|"filter"|"delay"|"reverb"|"crush"|"resonance" )
         | "master" ( "level"|"filter"|"delay"|"reverb"|"crush"|"resonance" ) | "crossfader"
         | "section" | "bar" | "bar_in_phrase" | "phrase" | "gen" [ "of" NAME ]
         | "hits" "(" ROLE [ "," DECK ] ")"          (* hits in this bar *)
         | "playing" "(" DECK ")" ;
```

No loops, no calls: it is **total**, and it reuses the shape of `evolve`'s guards (`gen % 4 == 3`).

Each operand is **Score** (a function of the program text and the bar: `bar`, `section`, `gen`,
`hits(...)`, and every `Fold` param the *program itself* sets) or **Live** (what a hand on a fader did,
read from the mixer at the bar line). The parser types the whole condition: **Score ⊕ Score = Score**,
anything with a Live operand is Live. Then:

- A **Score** condition is *scanned*: `next_stop(from)` walks `frame_at` forward to the song's end (or the
  orbit/16,384-bar bound) and returns the first match **without playing**. That makes `continue` a *plan*:
  the debugger knows the stop bar in advance and prepares its slice nets in the preceding bar's apply
  window, exactly as an `eval` does. **A breakpoint is a scheduled transport event, not a reaction**; that
  is what determinism buys.
- A **Live** condition is sampled at each bar line against the live mixer; it stops on the *next* bar, one
  bar of latency, and the UI says so (a hollow dot, "live").

`deck A level > 0.8` is Score if the program sets `deck A: level …` and Live if only your hand does. The
distinction is by where the value comes from in this frame (the `Fold` fold has a writer or not), not by
a flag the user must set.

### 3.3 How to set them

| Surface | Gesture |
|---|---|
| Desktop / web / Android editor | **Gutter click** on a line toggles a line/token/control breakpoint (a red dot left of the line number, the IDE convention). Click a **section header** for "break at start of section" (right-click: `+N bars`). Click the **section timeline** (the bar under the header, §9.1) to place a bar breakpoint. |
| Keyboard (desktop) | `F9` toggle at the caret's line; `Cmd+F9` edit the condition. (Chosen to avoid the window's existing `Cmd+Enter`, `Cmd+Shift+F`, `L`, `Esc`; to be checked against `app.rs`' key map.) |
| Verb / CLI | `debug break at drop+8`, `debug break when "deck A level > 0.8" action=ask`, `debug break on gen 3`, `debug breaks`, `debug unbreak b3` (`djctl app debug …`, `djapp_debug`). |
| MCP | `debug` actions `break`, `breaks`, `unbreak` (§9.4). |

### 3.4 Where they live: outside the program (recommended)

**Recommendation: breakpoints are session state, not program text.** Concretely: a `DebugSession`
(`djlive::debug`, serialised by `djhost::debug_store` to `<support>/debug/<lineage-root>.json`) holds
them, keyed by the *root of the program's lineage* so they survive edits and forks.

| | In the program (`break` statement) | Outside (recommended) |
|---|---|---|
| CID | A `break` line is in the canonical text: **debugging forks the program**, and the same music has two CIDs. | Untouched. "Same text, same CIDs" holds for the music. |
| Totality / liveness | A program that stops the transport is a *program that depends on a debugger*. Anyone opening a link could be paused by it. | Impossible by construction (below). |
| Portability | Travels with the text. | Travels via an explicit export (below). |
| Diff noise | Shows in program diffs and forks. | Does not. |

**Structural enforcement:** the debugger verbs live in their own registry noun `debug`, marked
`program_callable = false` in `control_registry`. `btw check` of `do debug hold` (or any sugar for it)
fails with "debugger verbs are not part of the language". So "a program cannot pause itself" is a property
of the registry, not a convention.

**Portability without the CID cost:** `debug export` writes the breakpoints as a `#! break: …` header
block on a saved `.btw` (header lines are comments to the language, like `#! name`), and a possible later
comment pragma `#@ break when …` next to a statement for the "debugger statement" ergonomics. Both are
dropped from the canonical text. **Imported pragmas arrive disarmed** (ghosted, user arms them): a share
link carries comments as *unverified notes*, and a hostile link must not be able to pause your session
(question 1).

### 3.5 Where a stop lands

The break position is computed ahead of time (§3.2) and the debugger engages **on the bar line** at or
before it, in the previous bar's apply window. Sub-bar precision (stop *on* the step of a token) would
need an engine-scheduled pause; v1 stops at the bar line of the bar that contains it and the inspector
marks the step. Everything that follows (hold-loop, A/B) is bar-aligned anyway. Question 8 asks whether a
sub-bar stop is worth an engine change.

---

## 4. Pausing the music

### 4.1 Modes

The debugger owns a `Transport`:

```rust
enum Transport {
    Playing,
    Held   { win: Window },                    // loop a window of the program; program clock frozen at its start
    Paused { at: Pos },                        // silent; decks stopped on the beat
    Stepping { until: Pos, then: After },      // playing to a boundary, then Held/Paused again
}
struct Window { start: i64, len: u32, pre: u32 }  // loops [start - pre, start + len)
```

| Mode | What you hear | Program clock | Use |
|---|---|---|---|
| **hold** (default) | The window loops (default: the **pending bar**; `len` and `pre` widen it: `hold bars=2 pre=1` is the lead-in bar plus the pending bar). Controls are re-seeked at each lap, so a ramp restarts and a one-shot re-fires. | frozen: the program position stays `start`. | Deciding on an edit with the music still audible. |
| **pause** | Silence. Decks stop on the beat (the engine's deck `play on=false` path). | frozen | Reading the inspector; comparing notes against text. |
| **soft** (v2) | Sequencing stops; the FX bus keeps running so delay, reverb and filter tails decay for up to *T* ms. | frozen | A musical stop. Needs an engine capability I have not verified (S3). |

Hold is the answer to "keep hearing context": there is no engine stop at all, only a different net on the
decks and a different number in `Live`.

### 4.2 The virtual program clock

`Live::service` currently does `self.line += lines_crossed(prev, g.beat)` where `g` is the host's grid.
The change is one indirection:

```rust
program_line = grid_line - shift          // shift: i64, was always 0
```

- **hold**: at the grid bar line where `program_line` would reach `start + len`, set `shift += len`, so it
  returns to `start`. Slice nets loop by themselves, so there is nothing to reload per lap (§2.4).
- **seek(N)** while playing (a "locate"): at the next bar line set `shift` so `program_line = N` and
  install `frame_at(N)`'s nets.
- **resume**: always lands on a **bar line**, never mid-bar. The program position becomes the **pending
  bar** `N` (the debugger semantics: "stopped before executing N"), which is a seek to `N` if the hold was
  mid-lap. The transport shows "resuming on the next bar" with the beats left.
- `watch_fires`'s "a jump is not a firing of everything in it" guard (`pos - prev < 1.5`) already makes a
  seek safe for the markers; they simply wait one pass.

`LiveHost` does not change. `status()`/`marks()` gain a `transport` block.

### 4.3 Installing a frame

`install(frame)` is: for each deck, load `slice` (hold) or `rotated(phase)` of the segment's net (resume,
seek); apply `Fold` params; re-execute `Replay` verbs; mark `Opaque` ones. Every load uses the existing
`load_net` and lands on the next bar line like any section net. Spike S2 confirms that the engine keeps
the loaded net's own phase from cycle 0 (so rotation is the only thing needed), and what `load_prepared`
does with a net whose CID is already loaded.

### 4.4 Step

| Step | Plays | Then |
|---|---|---|
| `step beat` | one beat | **pause** (a hold of one beat is a stutter, not an inspection) |
| `step bar` | the pending bar once | hold at the next bar |
| `step phrase` | to the next multiple of 8 | hold |
| `step section` | to the next section start | hold |
| `step back bar|phrase|section` | seek to the previous boundary, play it once | hold |
| `continue` | on, to the next breakpoint (computed, §3.2) | stop there |
| `run-to <anchor>` | a temporary breakpoint (`once`) | stop there |
| `reverse-continue` | seek to the previous breakpoint | hold |

`step back` and `reverse-continue` are free because seek is exact; they are the part of rr that usually
costs a recording.

### 4.5 Time travel, and its honest limits

`seek <anchor>` where an anchor is `bar N`, `drop+8`, `gen 3`, `song end`, or "the bar this breakpoint
fires on". Seeking works while playing, held or paused; while an edit is staged it first applies or
refuses. What is exact: nets, alternation phase, hits, evolve generation, every `Fold` param, every timed
line's fired/waiting/ramping/held state. What is not, by §2.3: `Opaque` verbs and the history-dependent
corner of §1.2. Both are in `Frame.fidelity` and shown in the inspector header (`exact`, or `approximate:
lines 14, 22`).

---

## 5. Inspect

### 5.1 The inspection

`debug inspect` (MCP `inspect`) answers a structure; the UI is a rendering of it.

```json
{
 "transport": {"mode":"held","window":{"start":23,"len":1,"pre":0},"since_bar":23,"resumes":"next bar line"},
 "run": {"cid":"z4EBG9j…","origin":0,"shift":-9,"fidelity":"exact"},
 "position": {"bar":23,"step":0,"phrase":3,"bar_in_phrase":8,
              "section":{"name":"drop","bar":8,"bars":16},"generation":[["drop",2]]},
 "lines": {"active":[{"line":31,"start":1204,"end":1250}], "fired_this_bar":[…]},
 "decks": {"A":{"net":"z…","parts":[{"role":"hihat","cycle":"1/2","hits":[{"step":6,"note":42,"vel":71,"sid":14}],
                                     "marking":{"ring":32,"token":22,"enabled":[…]}}]}},
 "controls": [{"key":"master: filter lp 300 -> 2800 over 1 bars","line":36,"state":"ramping","progress":0.0,"value":300,
               "next_fire":24}],
 "params": {"inserts.Master.filter.sweep":0.31,"mixer.A.volume":0.85},
 "evolve": [{"scope":"drop","gen":2,"every":8,"next_gen_at_bar":24,"rules_ran":["hihat: thin 0.1 seed=3"],
             "tail":34,"period":4}],
 "next_bar": {"changes":["hits +3 hihat A","control line 40 fires: move echo_out on A over 4 bars","gen 2 -> 3"],
              "frame_digest":"…"},
 "breakpoints": [{"id":"b1","state":"resolved","bar":24},{"id":"b2","state":"orphan","why":"section `break2` is gone"}]
}
```

- **Section, bar, phrase, generation, active lines, fired controls, ramps in flight**: from `Frame`
  (`Marks` today derives the same things from a live position; it becomes a view of `Frame`, so the panel
  and `inspect` cannot disagree).
- **Mixer and inserts**: `Frame.params` (what the *program* set) beside the live mixer reading
  (`mixer`/`inserts` `state`). A difference is a finding: "you moved deck A's fader; the program says
  0.85".
- **What would sound in the next bar** = `frame_at(N+1)` diffed against `frame_at(N)`: the hits that
  appear or vanish, the controls that fire, the section or generation that turns. It is a diff of two
  pure values, not a prediction.

### 5.2 The net marking ("show tokens")

A deck is a Petri net: per part a ring of places (one per sixteenth, per bar of the part's alternation)
and a transition per hit. The marking at a frame is "which place holds the token" per part ring, plus
which transitions are enabled (the hits about to fire).

- **From the IR, everywhere:** `PartFrame.marking = { ring: bars*16, token: step_of_pos, enabled: [...] }`
  is computed from `cycles` and the position, so it works headless, on the web, on Android, and for any
  seeked bar.
- **From the engine, on the desktop:** the deck's marking mirror (`readMarking` / `markingLayout`,
  `djrust/src/host/deck/views.rs`) reports what the engine itself holds. The inspector draws the IR
  marking and **flags disagreement** ("engine marking differs on deck A, part bass") as a self-check on
  the whole slice/rotate machinery. Spike S1: confirm the mirror's layout for a slice net.
- **Drawing:** one row per part: the ring as a small strip of 16·k cells with the token as a filled dot
  and the `<a b c>` phase as `cycle 2/3`; hits as ticks. This is the same visual vocabulary as the
  firing markers, so the lit token in the text and the dot in the net are the same event.

### 5.3 "Why is this sounding" (and "why is this silent")

Select a token (or click a lit one) and `why` returns the **derivation path** from the sound back to the
text. Today a `Fire` carries the token's spans and its call sites; the compile step must also record the
**transform steps** it passed through and the **evolve rule** that kept it. Output:

```
hihat, deck A, bar 24 (drop 9/16), step 6, note 42, vel 71
 1 token      x                          31:41   slot 3 of `hats`
 2 let        hats = ~ ~ x ~ …           4:5
 3 part       hihat hats vel=50..80      31:3    deck A, in section drop
 4 call       groove(A, 16)              36:3    expanded 1 time
 5 section    drop (bars 16–31)          34:1    arrange line 63
 6 transform  evolve drop gen 2: `hihat: thin 0.1 seed=3`   kept this hit (hash(seed=3, gen 2, bar 24, step 6, note 42))
 velocity 71 = hash(50..80, section bar 8, step 6)
```

**Why is this silent** answers the other half, from the frame's control table and the segment's part list:
"deck B is silent: section `breakdown` has no part for it, so the deck is stopped there (line 43)";
"hihat on A is locked out by line 52 (`role_disable`, bars 24–26, put back on bar 26)"; "muted by `move
echo_out` (line 40), put back at the first phrase line after it ends (bar 32)". These are the failure
modes BTW.md already documents, finally answerable.

`Provenance` is **not** part of the canonical form or any CID (like spans today), and costs nothing at
runtime: it is a table beside `spans`.

---

## 6. Prompt in context

### 6.1 One pipeline, two ways to reach it

```
 selection + instruction ──► context bundle ──► [ model ] ──► Proposal(ops) ──► apply ─► check ─► footprint
        ▲                      (§6.2)         outside          (§6.3)                              │
        │                                     the app                                              ▼
   accept / reject ◄── audition A/B ◄── preview (hold-loop | offline render) ◄───────────── guardrails (§6.8)
        │
        ▼
   commit as fork (forkedFrom, provenance), lineage = undo (§6.6, §6.7)
```

The model sits **outside** the pipeline. That is the simplification that makes everything else uniform:

- **Driven** (v1): Claude (Desktop/Code) calls the MCP tools: `context` to read, `propose` with its own
  ops. The app spends no model tokens; the user's own session is the model.
- **Hosted** (v1.1): the Ask box calls a model: on the web through `POST /api/compose` in **edit mode**
  (§6.9), on the desktop/Android through a configured key. It posts the same `Proposal`.

Both reach the same `propose`, so guardrails, preview and commit are written once.

### 6.2 The context bundle `btw.debug.context/1`

Deterministic (sorted keys, no timestamps): the same inputs give the same bytes, so the prefix is
cacheable by the model provider.

```json
{
 "format": "btw.debug.context/1",
 "program":   { "cid": "z4EBG9j…", "text": "<source as evaluated, or canonical for a foreign program>",
                "canonical_cid": "z4EBG9j…", "line_hashes": ["c1f2", "9ab0", "…"] },
 "selection": { "kind": "token|line|range|section|bar", "start": 1204, "end": 1250,
                "line": 31, "col": 41, "text": "hihat hats vel=50..80", "statements": [31] },
 "position":  { "bar": 23, "step": 0, "section": "drop", "bar_in_section": 8, "bars_in_section": 16,
                "phrase": 3, "bar_in_phrase": 8, "generation": [["drop", 2]] },
 "paused":    { "mode": "held", "window": {"start": 23, "len": 1, "pre": 0}, "fidelity": "exact" },
 "frame":     { "…": "the compact inspection of §5.1, trimmed to what this selection touches" },
 "why":       [ "… the derivation path of the selected token(s), §5.3" ],
 "instruction": "drop the hats for 2 bars",
 "policy":    { "scope": "window", "window": {"from": 23, "to": 25}, "allow_widen": false },
 "limits":    { "max_statements_changed": 12, "max_bytes": 2048 },
 "instruments": ["…", "from `btw instruments`"],
 "stdlib":    { "import": "std/transitions", "cid": "z…", "index": [ {"name": "drop_out", "sig": "drop_out(deck, role, bars)", "doc": "…"} ] },
 "grammar":   "<the compact grammar cheat-sheet: `btwd cheatsheet`, generated from docs/BTW.md>"
}
```

Ordering for prompt caching: **static prefix first** (system prompt, grammar cheat-sheet, instruments,
stdlib index), then the program, then the position/selection/instruction. `grammar` is the same text
the compose route already serves as `/docs/cheatsheet.md` (`btwd cheatsheet`), so the model sees one
grammar everywhere.

**Source vs canonical:** a program the user wrote in this session or saved as their own goes in **as
written, comments included** (comments carry intent). A **foreign** program (opened from a link, an
import's body, unverified notes) goes in **canonical, comments stripped**, and the program text is fenced
as data in the prompt. A comment in a stranger's program is an injection vector (R6).

### 6.3 The response: statement-anchored ops, not a rewritten program

```json
{
 "format": "btw.debug.proposal/1",
 "base_cid": "z4EBG9j…",
 "ops": [
   { "op": "insert_after", "anchor": { "line": 36, "hash": "c1f2" },
     "text": "at bar 24: drop_out(A, hihat, 2)" },
   { "op": "import", "name": "std/transitions" }
 ],
 "summary": "drop the hi-hats on bars 24 and 25 of the drop"
}
```

Op set: `replace`, `insert_before`, `insert_after`, `delete`, and `import` (add a pinned import once). An
**anchor** is a *statement* (a line with its `|` continuations, or a block header with its block) plus a
4-hex hash of that statement's text as a **precondition**. Why not a unified diff or a whole program:

| Format | Verdict |
|---|---|
| Whole program (the compose route today) | Forces the model to retype 60 lines to change 1: it drifts, and the diff is whatever it produced. Rejected for debugging. |
| Unified diff | Context lines go stale after any edit; models get line arithmetic wrong; hunks can split a statement. |
| Byte/offset edits | Same, worse. |
| **Statement ops with a hash precondition** (this) | A statement is the language's own unit, so an op **cannot half-edit** one (the structural constraint). A stale anchor is an error, not a misplaced edit. A pending proposal can be **rebased** mechanically onto a program the user kept editing (re-anchor by hash; conflict only if the same statement changed). Cheap to emit. Ops are the same shape Claude Code's and aider's edit tools use, which models are good at. |

`apply(source, ops)` is a pure function in `djlive::edit`. The new text is then parsed and **canonicalised**
by the existing printer, so the result's CID is canonical by construction; **untouched statements keep
their source bytes** (only the touched statements are re-spliced by span), so comments and the user's
formatting survive.

### 6.4 `propose`: apply, check, footprint

```
propose(base_cid, ops, scope) ->
   1. base_cid == running program CID, else rebase(ops) or {error: "stale", rebased: false}
   2. apply(ops)                       -> new source        (anchor errors say which statement changed)
   3. check(new source)                -> diagnostics        (the compose route's repair loop, ≤ 3 rounds)
   4. footprint(P, P')                 -> see below
   5. guardrails (§6.8)
   -> { ok, cid, canonical, diagnostics, footprint, plan: runtime::diff(P, P').summary(), ops_applied }
```

**The footprint is semantic, not textual.** Text changed is a poor proxy for what *sounds* different: one
edited `let hats` changes the hats in every section that uses it. So:

- Compile P and P'. Compute `frame_at(p, b).digest()` and `frame_at(p', b).digest()` for every bar `b`.
- `footprint.bars` = the bars where they differ, as ranges per deck and per section.
- `footprint.statements` = canonical statements added/removed/changed.

The **scope policy** (`window`, `section`, `song`) is checked against `footprint.bars`: a "make this bar
halftime" proposal whose footprint is bars 0–63 (because the model edited a shared `let`) fails with a
diagnostic *fed back to the model in the repair round*: "your edit changes bars 0–63 because `let hats`
is shared; change only the selection (use a section-local part or a std call)". The model is told what it
did, in the units that matter.

### 6.5 Preview

Two previews, same window `[from, to)` (default: the held window widened to **4 to 8 bars**, setting
`preview bars=N`).

1. **Live hold-loop (v1).** The proposed program is evaluated into the *held* runtime: `Live.eval(P')`
   lands on the next bar line, and the window loops with the edit in place. Because the preview is a hold
   (§4), the controls are **re-seeked at every lap** (`frame_at(P', start)`), so a riser plays
   identically each lap instead of getting stuck at its end. To honour hot-reload history (§1.2) the
   *starting* state is produced by **replaying P on a virtual clock** to bar `start` (no audio: the
   runtime is pure over `LiveHost`, and a fake-clock host already exists in the tests), then evaluating P'
   there. This is what the user would have heard had they typed the edit at that moment.
2. **Offline render (v1.1).** `btw_render` plus `render_arrangement` renders `[from, to)` of P' (and of P)
   to a file: needed for Claude-driven sessions with nobody listening live, for the web's `btwd render
   --from-bar N --max-bars k`, and for Android. **Prerequisite P0a:** control lines in the offline render.
   The render is deterministic ("composes run inline"), so previews are **content-addressed**:
   `(cid, from, bars, sr)` is the cache key, the same scheme as the site's render cache.

A cold window render lacks the reverb/filter tail of the preceding bars. The offline preview therefore
takes a `pre_roll` of 1 to 2 bars, discarded from the output, so the first audible bar sounds as it does
in context.

### 6.6 A/B audition

`audition` holds the window and alternates the two programs **on bar lines**, controlled by `a`, `b` or
`toggle` (and `alternate`, which swaps each lap):

- It is a hot-reload swap `P <-> P'` at a bar line with the control state **re-seeked** at the lap start,
  so both play from the same frame and neither inherits the other's ramp state. No glitch beyond what a
  section change has.
- **Level-matching** (v1.1): loudness differs between the two (a riser is *louder*, and a louder
  candidate is heard as better regardless of merit). The offline preview measures integrated loudness
  (`docs/POST.md` already has LUFS) and the audition offers a "match levels" trim on the master.
- Verdicts: `accept`, `reject`, or `refine "less reverb"` (a new instruction against the *proposal* as the
  base: a chain of small ops, each a fork).

### 6.7 Accept, lineage, undo

- **Accept** = `Live.eval(P')` as the running program (lands on the next bar line) **and** a commit:
  `live_store` writes P' as a fork (`forkedFrom = base CID`, license and credits carried, **private by
  default** until saved or shared; the compose route's `CC0-1.0` default applies only to web composes).
  Provenance rides in the object's `#!` header, as `composed-by` does on the site, and is *not* in the
  canonical text or the program CID:

  ```
  #! forked-from: z4EBG9j…
  #! edit: drop the hats for 2 bars
  #! edit-by: claude (mcp) | gemini-3.8-flash (compose)
  #! edit-window: drop+8 .. +2 bars
  #! edit-footprint: bars 24-25 deck A hihat
  ```

  The **instruction is the commit message.** (Open question 6: whether *every* accept persists, or the
  session keeps an in-memory lineage until `save`.)
- **Lineage is the undo stack.** The session keeps `lineage: [CID]`; `undo` evals the parent (lands on the
  next bar line, via the same hot-reload diff), `redo` the child; `history` lists
  `(cid, instruction, window, when)`. Undo then a new edit makes a **branch**: `forkedFrom` is already a
  tree. Because programs are content-addressed and tiny, every node is storable.
- **Reject** drops the proposal; the program and the transport are untouched (the hold simply carries on).
  Rejected proposals stay in the session history with their CIDs, so "the one before the last" is
  recoverable.

### 6.8 Guardrails

| Guardrail | How it is enforced |
|---|---|
| Edit stays in the selection (or the declared scope) | **Footprint ⊆ policy window**, computed from frame digests (§6.4), not trusted from the model. `allow_widen` is an explicit user choice. |
| Program stays **total and deterministic** | By construction: any program that parses is total (no recursion, no unbounded loops, written-out counts). The pipeline additionally requires `check` ok, a **canonical fixed point** (`canonical(canonical(x)) == canonical(x)`), and that compiling twice gives the same CIDs. The bound checks (net ≤ 64 bars, timeline ≤ 16,384 bars, expansion ≤ 4096 lines) are the language's own. |
| The model cannot call the debugger from the program | `debug` is `program_callable = false` (§3.4). An op whose text contains such a call fails `check`. |
| Small edits | `max_statements_changed` (default 12) and `max_bytes` (default 2 KiB) from the bundle's `limits`; over the cap is refused with "ask for a smaller change". |
| No restricted / private / asset material reaches a **hosted** model | The bundle builder refuses a program with `use restricted`, `sample`/`loop` assets, or `visibility: private` for any hosted model (the site's rule: "never send private or restricted assets to the model"). The MCP-driven path is the user's own session; the app does not send anything on the user's behalf there. |
| Prompt injection from program text | Foreign programs go in canonical (comments stripped) and fenced as data; ops are *data the pipeline validates*, and an op cannot do anything but edit statements. |
| Header and rights untouched | Ops cannot target `#!` lines, `license`, `use restricted`, or `sample` statements. |
| Rate and cost | **Hosted:** every call goes through the compose route's `limits` (user/hour, user/day, IP/day, the daily spend cap and kill switch), each repair round counted. **Local/MCP:** a per-session cap on `propose` calls and `max_repair`. |
| Stale base | `base_cid` must equal the running CID or the ops must `rebase` cleanly. |

### 6.9 The compose route: edit mode

`POST /api/compose` today takes `{prompt, genre, bpm, key, length, base_cid}` and returns a whole program.
For debugging it grows an **edit mode** in the same handler (same auth, same limiter, same repair loop):

```json
{ "mode": "edit", "prompt": "drop the hats for 2 bars", "base_cid": "z…",
  "selection": { "start": 1204, "end": 1250 },
  "window": { "from": 23, "to": 25 }, "scope": "window",
  "frame": { "…": "§5.1 compact" } }
```

Response adds `ops`, `footprint`, `applied` (the canonical program) and keeps `cid`, `forkedFrom`,
`attempts`, `cost_usd`, `usage`. Constraints already in the route stay: auth required (cookie or bearer),
`base_cid` must be a **public CC0 unrestricted** program in the store (`PUT /api/programs/<cid>` first;
private programs are simply not editable by the hosted model), and `max_prompt_chars` bounds the
instruction. Edit calls are small (selection plus a frame, not a whole generation), so they can be given a
lower weight in the limiter; that is a tuning decision for Matt, not a change to the scheme.

---

## 7. Transitions as a standard library

### 7.1 Do they belong in `std/transitions`? Yes

The model should call **named transforms** rather than write raw ramps, for four reasons:

1. **Small diffs.** `at bar 24: riser(4)` is one statement; the raw form is four lines and four places to
   be subtly wrong.
2. **Readable provenance.** The `why` view (§5.3) and the commit message say `riser`, which is what the
   user asked for.
3. **Compositional.** Defs call defs (non-recursively), and a call's time word shifts the whole body, so
   `before drop 4 bars: build(4)` composes `riser` and `loop_roll`.
4. **Pinned.** `import "std/transitions"` becomes `import z…` in the canonical text, so the same program
   is the same program for good, and a new version is a new CID, not a silent change.

`import` already brings **lets and defs only**, which is exactly a library. **No grammar change is
needed for v1.**

### 7.2 Proposed contents (sketches: each must pass `check` in P6)

These are sketches written against BTW.md and the registry as read, **not yet checked**. The point is the
shape. Time words compose as in `expand.rs`: the call's word is the body's base, and inside a body
`at bar N:` is relative to it.

```text
# std/transitions v1 (CC0). Imports bring lets and defs only.

# a build: the master high-passes open and the reverb swells over `bars`, then both are put back
def riser(bars) {
  master: filter hp 100 -> 5000 over $bars bars
  master: reverb 0.1 -> 0.5 over $bars bars
  at bar $bars: master: filter off
}

# a sweep on one target, either direction
def filter_sweep(target, kind, from, to, bars) {
  $target: filter $kind $from -> $to over $bars bars
}

def echo_out(deck, bars)  { move echo_out on $deck over $bars bars }
def loop_roll(deck, bars) { move loop_roll on $deck over $bars bars }

# the deck's clock at half speed for `bars`, put back by `for`
def halftime(deck, bars) {
  for $bars bars: trick on deck=$deck kind=half_time
}

# a role locked out for `bars`, then back
def drop_out(deck, role, bars) {
  do deck role_disable deck=$deck role=$role on=true
  at bar $bars: do deck role_disable deck=$deck role=$role on=false
}

# the tape stops
def tape_stop(deck) { trick on deck=$deck kind=tape_stop }

def build(deck, bars) {                       # composition: a riser plus a roll
  riser($bars)
  at bar $bars: loop_roll($deck, 1)
}
```

| Name | The user's phrase | Built from | Note |
|---|---|---|---|
| `riser(bars)` | "add a riser / build into the drop" | master `filter hp` ramp, `reverb` ramp, put-back | bar-aligned on the call's base |
| `filter_sweep(target, kind, from, to, bars)` | "sweep the filter" | one ramped `filter` | `target` is `master:` or `deck A:` |
| `echo_out(deck, bars)`, `loop_roll(deck, bars)` | "echo it out", "roll it" | the existing `MOVE`s | strip put-back is the language's own rule |
| `halftime(deck, bars)` | "make this bar halftime" | `trick half_time` + `for` | **deck-clock** halftime; a *pattern* halftime is the gap below |
| `drop_out(deck, role, bars)` | "drop the hats for 2 bars" | `deck role_disable` on/off | `Replay`-class, so seekable |
| `tape_stop(deck)`, `spin_back`, `stutter(deck, beats)` | stops and stutters | `trick` kinds | |
| `build(deck, bars)` | "a build" | `riser` + `loop_roll` | shows composition |
| `key_lift(n)` | "lift the key" | **not expressible today** | see 7.3 |

### 7.3 Gaps this exposes (for Matt)

Three of the user's own examples are not purely in-language today, and the debugger will hit them every
session:

- **G1. A bar-ranged override of a *part*.** "Drop the hats for 2 bars" is expressible only through the
  registry (`role_disable`: engine state, `Replay`-class) or by **splitting a section** (changes the
  arrangement). Neither is a pure function of the score. `every(n, p, q)` is periodic and has no offset.
- **G2. A scoped pattern transform.** "Lift the key", "thin the hats for a bar", "reverse this fill",
  "halftime this *pattern*" want `transpose`/`thin`/`reverse`/`slow` applied to *existing parts* for a
  literal bar window.
- **G3. Untyped def parameters.** `riser(A, 4)` vs `riser(4, A)` fail at expansion, with a trace. Typed
  params (`def riser(bars: int)`) would put the error at the call site, where a model will make it.

**Proposed one-feature answer to G1 and G2, for Matt to decide (question 3):** an `overlay` statement, a
sibling of `evolve` that applies the *same* endomorphisms (`rotate`, `reverse`, `transpose`, `octave`,
`invert`, `thin`, and `drop`) to named parts over a literal bar window:

```text
at bar 24 for 2 bars: overlay hihat: drop                 # the hats out, as a function of the score
at bar 31 for 1 bars: overlay deck A: reverse | thin 0.5 seed=4
```

It is total (literal windows), deterministic (seeds, hash-picked), pure (so `Fold`-class and **seekable**
exactly), and it gives `drop_out`, `key_lift`, `halftime` (pattern form) a home in the library without
touching engine state. It is a grammar and canonical-form change and so out of v1.

### 7.4 Pinning and versioning

- The library is **shipped text** under `crates/djhost/std/transitions.btw`, embedded with `include_str!`.
  A golden test asserts `cid(text) == STD_TRANSITIONS_V1`, so the library cannot drift silently.
- A change is a **new version and a new CID** (`std/transitions@2`); **old versions stay embedded** so old
  imports keep resolving (content-addressed: the CID *is* the dependency).
- The web needs the same text: embedded in `btwapi`/`btwd`, seeded into the programs store, served at
  `/docs/…`. Android gets it through `djmobile` (`rust/src/btw/`, `include_str!`).
- The context bundle carries the library's **index** (name, signature, one line) pinned to that CID, and
  the system prompt says: *prefer a std call; write raw statements only when none fits; do not edit or
  re-declare a std name.*
- A later lint nudges the model: a raw statement that is the expansion of a std def draws a hint ("this is
  `riser(4)`").

---

## 8. Architecture: where each piece lives

```
 djlive (pure, wasm-clean)                      djhost                      surfaces
 ─────────────────────────                      ──────                      ────────
 timeline.rs   fires_on, ramp_value  (P0b)
 frame.rs      Pos, Frame, frame_at, digest
 debug.rs      Breakpoint, Anchor, Cond,        debug_store.rs  sessions    djapp   live_panel.rs  gutter, transport,
               Debugger, Transport, Window,     btw_env.rs     `debug`              inspector, Ask, diff, A/B
               next_stop, rebind                control_registry/nouns.rs           control/debug.rs  the verbs
 edit.rs       Op, apply, rebase, footprint     btw_render.rs  windows + control    djctl   btw frame | propose (headless)
 context.rs    bundle builder (deterministic)                  ops (P0a)            djweb   BtwRun debug exports (wasm)
 runtime.rs    `shift` clock, install(frame)    live_store.rs  forks, lineage       djmobile rust/src/btw/debug.rs (uniffi)
 compile.rs    slice/rotate, Provenance         std/*.btw      the library          MCP    djapp_debug / site /mcp debug_*
```

The rule that decides placement: **anything that is a function of (text, bar) goes in `djlive`**, because
`djlive` is the one crate every surface already compiles (desktop, the wasm module, the Android lib).
`djhost` holds what touches storage and the registry. Surfaces own only drawing, input, and the clock.

---

## 9. Surfaces

### 9.1 Desktop: the btw window, and full screen

```
┌ btw ─ ▸ drop 9/16 · bar 24 · phrase 3 · ● ● ● ● ● ● ● ○ · gen 2 ──────── [Full] [Follow] ┐
│ ⏸ HELD  window: bar 24 (1) · exact          [▶ resume] [⏭ bar] [⏭ phrase] [⏮ back] [seek ▾] │
│ ├intro──┼build─┼─────drop────[█████▓░░░░░░░]┼breakdown┼───drop2────┼outro┤   ◂ ticks = breaks │
│ 28 ●│   deck A: kick kicks | snare snares | hihat hats vel=50..80           ◂ ● breakpoint   │
│ 31 ▸│   hihat …  x ~ [x] ~ …        why ▸ token ← let hats ← part ← groove(A,16) ← drop       │
│ 36 ○│   at bar 24: …                                       (○ orphan / live condition)        │
│ ┌ Inspect ──────────────────────────────┐ ┌ Ask ─────────────────────────────────────────────┐ │
│ │ A  kick ●···|···· snare ····|●··· …   │ │ ⌖ hihat hats (line 31) · bar 24     [window ▾]  │ │
│ │ B  harmony cycle 2/4                   │ │ > drop the hats for 2 bars                 ⏎   │ │
│ │ controls  ▰▰▱▱ filter lp… 0.31         │ │ ── proposal (1 statement) ───────────────────── │ │
│ │ next bar  +3 hihat · gen 2→3           │ │ + 36  at bar 24: drop_out(A, hihat, 2)           │ │
│ └────────────────────────────────────────┘ │ footprint: bars 24–25 · deck A hihat · ok       │ │
│                                             │ [▶ preview] [A | B | alternate] [accept] [reject]│ │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Gutter**: the existing dots stay (green playing, amber just-fired, teal control, red held). A breakpoint
  is a red-ringed dot in the new leftmost column; a conditional one has a `?`; a live condition is hollow;
  an orphan is dimmed with a tooltip. The **section timeline** under the header gets break ticks and is
  draggable to seek (a scrub that is a locate to a bar line).
- **Follow** keeps working: while held, the follow target is the *hold window's* lines, so the editor
  scrolls to the section you stopped in and stays (`btw_follow.rs`'s `Frame` input gets the held position).
- **Full screen** (`Cmd+Shift+F`): the stage stays dark and large. The transport strip stays on top with
  the header; the inspector collapses to the strip of net rings; **`Cmd+K` opens a one-line Ask overlay**
  (selection chip, instruction, `⏎`); the diff replaces the editor's selected region inline (green/red);
  `A`/`B` keys toggle the audition. Keys to confirm against the existing map: `F9` break, `F5` resume,
  `F10` step bar, `Shift+F10` step phrase, `F11` step section, `Cmd+K` ask, `Esc` leaves hold's Ask first
  and full screen second.
- The window already folds the plan under one line; the debugger **adds one collapsible block (Debug)**
  rather than a second panel, in the same style, to keep the audit shots of `layout_audit.rs` stable.

### 9.2 Web (btw.bitwrap.io)

The core is `djlive`, so `BtwRun` (the wasm `Live` plus a command-queue host) gains the same debug
methods, and the page is `driver.js` + `editor.js` + a new `debug.js`:

- **Gutter click** in `editor.js` (it already draws gutter dots), the transport bar in the slim bar,
  inspector and Ask in the same panel family as the Compose box. **The Ask box *is* the Compose box in
  edit mode**: same sign-in (wallet/Ed25519), same limits.
- **Two clock sources** exist on the page (the engine and the rendered stream; DESIGN.md). **Hold needs
  the engine**: a stream cannot loop a window of the *program*, and `btwd` does not render control lines.
  So a hold on the stream source switches to the engine (a one-line notice). Where only audio is
  available, a window of a *rendered* stream can still be looped client-side with a looping
  `AudioBufferSourceNode` (`loopStart`/`loopEnd`), for a symbolic-free A/B of two renders.
- **MCP**: the site's session bridge (`/api/session/<id>/events` SSE and `/report`) gets `debug` events
  and a `debug` block in the page's report, so a Claude on `/mcp` drives the page's debugger
  (`debug_break`, `debug_hold`, … below).
- **Deep link:** a share link may carry a **position** (`…#<CID>.<text>@drop+8`): opening it holds there.
  It makes "look at this bar, here is what I asked" a link. The position is outside the sealed part, like
  `~notes`. (Small, high value; question 13.)

### 9.3 Android (djandroid)

`djandroid/rust/src/btw/` already wraps `djlive` for the phone (`Player::btw_*`, spans as UTF-16,
`ComposeClient`, `BtwSheets.kt`). The debugger adds `rust/src/btw/debug.rs` (uniffi) over the same core and
a **Debug sheet** in the btw UI:

- Tap a **line number** to toggle a breakpoint; tap a **token** to select it (selection = what the Ask
  uses); long-press a section header for "break here / ask here".
- The transport row is thumb-sized: ⏸ hold, ⏭ bar, seek slider with section blocks.
- Proposals appear as a diff sheet with `A/B` as large toggle chips (touch targets per DESIGN.md §1:
  playable controls on play screens), and the compose client sends the edit-mode request.
- **Constraints:** slice/rotate compile and `frame_at` run on a worker, never on the audio-adjacent thread;
  the Android build is published only from one build box (its debug key signs the APKs). A Claude-driven session on
  the phone has no MCP server of its own today; it goes through the site's session bridge (question 12).

### 9.4 MCP and CLI: so Claude can drive a debug session

On the desktop these are actions of one registry noun, **`debug`** (so `djapp_debug`, `djctl app debug`,
and `do debug …` in the *control language* for tooling, but **never callable from a btw program**). On the
site they are flat tools with the same names, `debug_<action>`.

| Action | Args | Returns |
|---|---|---|
| `break` | `at` \| `when` \| `on` (bar, section±offset, token `line:col`, control, gen, condition), `action` (hold/pause/log/ask), `hits`, `once`, `label` | the breakpoint, resolved bar or `orphan` |
| `breaks` | – | all breakpoints with their state |
| `unbreak` | `id`, or `all`; `enable`/`disable` | the list |
| `pause` | `mode` (hold/hard/soft), `window` (`bars`, `pre`) \| `at` (an anchor) | transport + the inspection |
| `resume` | `at` (default: the pending bar) | when it lands (`lands`: bar line) |
| `step` | `unit` (beat/bar/phrase/section), `back` | the stop position and inspection |
| `continue` | `reverse` | the next breakpoint's bar and why it stopped |
| `seek` | `to` (any anchor) | the frame's inspection, `fidelity` |
| `inspect` | `section` (all, or one of: position, decks, controls, params, evolve, next) | the structure of §5.1 |
| `why` | `at` (selection or `line:col`), `silent: true` for the other direction | the derivation path of §5.3 |
| `context` | `selection`, `instruction`, `scope`, `window` | the bundle of §6.2 (so the caller can inspect what a model would see) |
| `propose` | `base_cid`, `ops`, `scope`, `instruction`, `summary` | `{ok, cid, canonical, diagnostics, footprint, plan}`, a **pending** proposal id |
| `preview` | `proposal`, `bars` (4 to 8), `mode` (loop/render) | loop started, or a render file with `cid`, `seconds`, `loudness` |
| `audition` | `proposal`, `which` (a/b/toggle/alternate), `match_levels` | which is sounding |
| `accept` | `proposal` | the new running CID, `lands`, the committed fork |
| `reject` | `proposal` | – |
| `undo` / `redo` / `history` | – | lineage nodes |

**A session, as Claude drives it** (Matt: "make the second drop's first bar halftime"):

```
debug break      {at: "drop2", action: "hold"}          -> b1 resolved bar 40
debug continue                                           -> stopped at b1, bar 40, held (window 40, 1 bar), exact
debug inspect                                            -> section drop2 1/16, deck A kick/snare/hihat/bass, filter ramps off
debug context    {selection: "bar", instruction: "make this bar halftime", scope: "window"}
                                                         -> bundle (stdlib index lists halftime(deck, bars))
debug propose    {base_cid, ops: [{op: insert_after, anchor: {line: 54, hash: "9ab0"},
                                   text: "at bar 40: halftime(A, 1)"}, {op: import, name: "std/transitions"}]}
                                                         -> ok, footprint bars 40-40 deck A, plan "+1 control"
debug preview    {proposal: p1, bars: 4}                 -> looping bars 39..42 with the edit
debug audition   {proposal: p1, which: "alternate"}     -> A,B,A,B per lap
   … Matt listens …
debug accept     {proposal: p1}                         -> z4EBG…9m, lands on the next bar line, fork recorded
```

`djctl` offline (no app): `djctl btw frame FILE BAR` (the inspection of any bar of any program, headless),
`djctl btw break-scan FILE "when …"` (the bars where a score condition holds), `djctl btw propose FILE
ops.json` (apply, check, footprint). These alone let Claude *debug a program with no audio device*,
which is useful on a headless host and in CI.

**A DAP adapter** (later): the verbs above map onto the Debug Adapter Protocol (breakpoints with
`condition`/`hitCondition`/`logMessage`, `continue`, `next`, `pause`, `stackTrace` = the `why` path,
`scopes`/`variables` = the frame), which would give VS Code and Zed a debugger for `.btw` files for free.

---

## 10. Implementation plan

### 10.1 Prerequisites (separate small branches, each independently useful)

| | What | Why |
|---|---|---|
| **P0a** | **Control lines in the offline render**: an `ArrangeOp` for registry calls (`mixer`, `inserts`, `trick`, `deck`), `btw_render` emits them from `Compiled.controls`, with `from_bar`/`to_bar`. | Without it an offline preview of a riser or filter sweep is silent, and the site's `btwd render` shares the limitation. Blocks §6.5.2, not v1. |
| **P0b** | Move timing/ramp/put-back logic out of `Live::fire_bar` / `move_ramps` into `djlive::timeline`, called by both. | §2.2: seek equals played by construction. Pure refactor, existing tests must stay green. |
| **P0c** | Seek classes (`Fold`/`Replay`/`Opaque`) on `control_registry::Action`; `program_callable` on nouns. | §2.3, §3.4. Default `Opaque` / callable, so nothing changes until classified. |

### 10.2 Phases

| Phase | Delivers | Crates and files |
|---|---|---|
| **P1 time travel, headless** | `Frame`, `frame_at`, `digest`, `Fidelity`; `djctl btw frame`; `debug inspect`/`seek` in *dry* mode. **Useful on its own** to Claude and CI. | `djlive/src/frame.rs`, `timeline.rs`; `djctl` (`btw frame`); `tests/frame.rs` |
| **P2 transport** | `shift` clock, `Window`, `slice`/`rotated`, `install(frame)`, hold/pause/resume/step/seek on the desktop; `transport` in `status()`/`marks()`; follow stays on the held window. | `djlive/src/runtime.rs`, `compile.rs` (`DeckIr::slice/rotated`), `debug.rs`; `djapp/src/{live_panel.rs, btw_follow.rs, control/debug.rs}`; `djhost` registry noun `debug` |
| **P3 breakpoints** | Kinds, anchors, `Cond` parser and typing, `next_stop`, `rebind`/orphans, session store, gutter and timeline UI, `debug break*` verbs. | `djlive/src/debug.rs`; `djhost/src/debug_store.rs`; `djapp/src/live_panel.rs` |
| **P4 inspector** | `Inspection`, marking from IR + mirror cross-check, `Provenance` (transform and evolve-rule steps recorded in compile), `why` and `why silent`, the Debug block in the window. | `djlive/src/compile.rs` (Provenance), `frame.rs`; `djapp/src/live_panel.rs`; `crates/djhost` marking adaptor |
| **P5 edit loop (MCP-driven)** | `edit.rs` (ops, apply, rebase, footprint), `context.rs`, `propose`/`preview` (hold-loop)/`audition`/`accept`/`reject`/`undo`, lineage in `live_store`, provenance headers; `djapp_debug` verbs complete. | `djlive/src/{edit.rs, context.rs}`; `djhost/src/live_store.rs`; `djapp/src/control/debug.rs`; `djctl` (`btw propose`) |
| **P6 std/transitions** | The library, golden CID, `check` harness for each def, index in the bundle, cheat-sheet and system-prompt text, `import` resolution in `btw_env`. | `djhost/std/transitions.btw`; `djhost/src/btw_env.rs`; `btwapi` cheatsheet |
| **P7 web** | `BtwRun` debug exports, `debug.js`, gutter and Ask in the page, `/api/compose` edit mode, `debug_*` MCP tools, session-bridge events, deep-link position. | `crates/btwapi`, `crates/djweb` (on `feat/btw-web`); beats `internal/btwsite/{compose,mcpsrv,limits,api}`, `btwsite/web/js/{debug.js,editor.js,driver.js}` |
| **P8 Android** | `rust/src/btw/debug.rs`, Debug sheet, edit-mode compose client. | djandroid `rust/src/btw/`, `BtwSheets.kt`, `ComposeClient.kt` |
| **P9 later** | Offline preview (needs P0a) and level-matching; soft-stop; sub-bar stops; `overlay`; typed def params; DAP adapter. | engine and grammar changes, each its own design |

### 10.3 Spikes to run first (they decide risk, and take hours)

- **S1.** What does the engine's marking mirror report for a one-bar slice net and a rotated net? Does it
  agree with the IR marking?
- **S2.** How does `Engine::load_prepared` choose a loaded net's starting cycle, and what does it do with a
  net whose CID is already loaded? (Decides whether rotation alone is enough; §4.3.)
- **S3.** Can the engine stop sequencing while the FX bus keeps ringing (soft-stop), and can a stop be
  scheduled on a beat rather than a bar line?
- **S4.** Which registry verbs `btw_render` can already turn into `ArrangeOp`s (the HEADLESS table lists
  `param`, `fx`, `trick`, `vocoder`, `freeze` ops for *logged sessions*; the btw side just never emits them).

### 10.4 Tests

Everything pure is tested on the **fake clock** the repo already uses (`tests/btw_runtime.rs`,
`marks.rs`, `hot_reload.rs`).

| Test | Asserts |
|---|---|
| **Seek equals played** (`tests/frame.rs`) | For `hand-dubstep-140-v2`, `trance-138`, `markers-demo`, the evolve fixtures: run `Live` + a recording `LiveHost` on a virtual clock from bar 0 to the end; at *every* bar line `frame_at(c, b)` equals the state derived from the recorded commands (net per deck and phase, fired control set, ramp values within `RAMP_EPS`, held strips, generation). Then the same over **randomly generated programs** (the generator `canonical.rs` already uses, extended with `every/at/after/before/for/evolve`). |
| **Seek then play equals play** | `seek(N)` then run to `M` produces the same symbolic command log as the continuous run's `[N, M)` (modulo the `rot`/`slice` net tags). Audio, once P0a exists: the window render equals the same slice of the full render after a 2-bar pre-roll, within a residual bound (the tails are the only legitimate difference). |
| **Hold and resume** | During a hold the program position is constant; after `resume`, frames equal a continuous run from `N`, **including** an `<a b c>` part and a hold length that is not a multiple of 3 (the rotation case). |
| **Breakpoint scan** | `next_stop` equals brute-force "play through and test each bar" for random Score conditions; anchors **rebind** after an edit (`drop+8` after inserting a section before `drop`); a deleted section gives an orphan, never a silent drop. |
| **Condition typing** | Score vs Live is decided from the frame's writers; a mixed condition is Live; the grammar is total (a fuzz over the EBNF never loops or panics). |
| **Ops and CID stability** | Random single-statement `replace`/`insert`/`delete` over the corpus: the nets of every deck and segment that does not contain the statement keep their CIDs; untouched statements keep their source bytes; `apply(inverse(ops))` returns the original *canonical CID* (the undo law); a stale hash is an error. |
| **Footprint** | The footprint equals the brute-force set of bars whose digests differ; a shared-`let` edit's footprint is the song, and the `window` policy rejects it with the repair message. |
| **Determinism** | Same `(program, selection, instruction, window)` gives byte-identical context bundles; same `(text, ops, window)` gives the same proposal CID and, with P0a, the same preview file hash. |
| **Structural bans** | `do debug …` and every sugar for it fail `check` ("not part of the language"); an op cannot touch `#!`, `license`, `use restricted`, `sample`. |
| **Std** | `cid(text) == STD_TRANSITIONS_V1`; each def `check`s in a harness program; expansion goldens (net CIDs) for each call; old versions still resolve. |
| **Registry round trip** (`app_tests.rs`) | Every `debug` verb through `djapp_debug` and `djctl app debug`, as `btw` verbs are today. |
| **Hosted edit mode** (Go, fake model) | good, repairable (footprint repair round), unrepairable → 422, auth refusal, rate limit, spend cap, kill switch, private/restricted base refused, stale base. |
| **UI shots** | `layout_audit.rs`: held, stepping, orphan breakpoint, proposal diff, A/B, full-screen Ask (the `--ignored` shot test pattern). |
| **Model eval** (not CI) | A corpus of ~30 (program, selection, instruction) triples run against the real model: ratio check-ok, footprint-in-scope, std-used, plus a human rubric for "did it do what was asked". Recorded like the compose route's `real-<date>.json`. |

### 10.5 Risks

| | Risk | Mitigation |
|---|---|---|
| **R1** | Offline render lacks control lines, so a preview of "add a riser" is silent. | v1 previews by **live hold-loop** (runs the real runtime and engine). P0a unblocks offline. |
| **R2** | Seek is "fresh-play", hot reload is "history-dependent"; the two disagree for `now` lines and ramps that survived an edit. | Previews **replay** old-to-N then apply the edit (§6.5.1); the inspector shows `fidelity`; the test suite pins the difference on a fixture. |
| **R3** | Engine behaviours I have assumed (phase on load, load of an already-loaded CID, sub-bar stop, soft-stop). | Spikes S1 to S3 before P2; v1 does not need sub-bar or soft. |
| **R4** | Slice/rotated nets cost a compile on the window thread at each seek or break. | Both are tiny (a bar or phrase of the cycles) and cached by CID; the debugger prepares them in the previous bar's apply window (§3.2), the same budget as a section load. |
| **R5** | The model produces valid, in-scope, **musically wrong** edits. | Small ops, std calls, A/B, `refine`, undo. This is a product risk the pipeline contains but cannot remove; the eval corpus tracks it. |
| **R6** | Prompt injection through program comments or names. | Foreign programs canonical-only and fenced; ops are validated data; the model has no tools here other than emitting ops. |
| **R7** | A private or restricted program leaks to a hosted model. | Refused in the bundle builder for any hosted model; the MCP-driven path sends nothing from the app. A test pins it. |
| **R8** | Cost abuse of the hosted edit mode. | The compose route's limiter and spend cap, repair rounds counted; edit weight tunable. |
| **R9** | A user keeps typing while a proposal is pending. | `base_cid` check plus statement-hash **rebase**; a conflict asks to re-propose rather than clobber. |
| **R10** | The btw window is already dense; a debugger can swamp it. | One collapsible Debug block, the Ask in the same panel family, layout-audit shots in CI. Full screen keeps only transport + `Cmd+K`. |
| **R11** | Android audio underruns from compile work. | Worker thread; frames and slices are tiny and cached. |
| **R12** | Scope creep into a general DAW transport. | The transport is *only* hold/pause/step/seek over the program clock; it never touches the sample decks or the setlist queue. |

### 10.6 Recommended v1 scope

**v1 = P0b, P0c, P1, P2, P3, P4 (without engine cross-check), P5 (MCP-driven, hold-loop preview), P6.**

In words: *a Claude-driven debug loop on the desktop*, with every guarantee the design is about and none
of the expensive extras.

- Headless `frame_at`/`seek`/`inspect` (also `djctl btw frame`).
- Desktop `hold`, `pause`, `resume` (bar line), `step beat|bar|phrase|section`, `seek`, `step back`.
- Breakpoints: bar, section±offset, line/token, control line, generation, **Score** conditions, session
  state, gutter + timeline UI. (Live conditions and `action: ask` can ride along; they are small.)
- Inspector: position, section, phrase, generation, active lines, controls and ramps, params, decks'
  parts and **IR markings**, `next bar`, `why` (token → source) and `why silent`.
- Edit loop: `context`, `propose` (statement-op, footprint, `check`), live **hold-loop preview**, A/B
  audition, `accept` as a fork with provenance, `undo`/`redo` over the lineage.
- `std/transitions` v1 (the table in §7.2, as `check`-verified defs).
- MCP/CLI complete; no hosted model.

**Deliberately not in v1:** the web and Android surfaces (v1.1, same core, §10.2 P7/P8), the hosted
compose edit mode (needs the site to land), offline preview and level matching (P0a), soft-stop, sub-bar
stops, the engine marking cross-check (needs S1), `overlay`, typed params, DAP.

**Why this cut:** it touches only `djlive`, `djhost` and `djapp`; **no engine (`djrust`) change, no grammar
change, no new service**. It makes the property that matters (deterministic time travel) real and
testable first, and it is usable the day P5 lands by the one user who already has a model in the loop:
Claude on the MCP the app already exposes.

---

## 11. Open questions for Matt

1. **Breakpoints outside the program (recommended) vs. a `#@ break` comment pragma** for portability.
   Pragmas are CID-neutral (comments are dropped) but a shared link carries comments as unverified notes,
   so imported pragmas would arrive *disarmed*. Want the pragma at all, or only `#! break:` on export?
2. **Default hold window.** The pending bar only (recommended: it is "the line about to execute"), or
   `pre=1` (the lead-in bar plus the pending one) so the edit is judged *in context*?
3. **Language gaps G1/G2/G3** (§7.3). Add `overlay` (bar-ranged transforms of existing parts; the clean
   home for "drop the hats for 2 bars", "lift the key", pattern-halftime) and typed def parameters, or keep
   the library on registry verbs (`role_disable`, `trick`) and accept the `Replay`-class seek? `overlay`
   is a grammar and canonical-form change.
4. **Edit format.** Statement-anchored ops with hash preconditions (recommended) vs. whole-program return
   (what `/api/compose` does today) vs. unified diff.
5. **Who is the model in v1?** MCP-driven only (recommended: Claude Desktop/Code *is* the model, no key in
   djapp), or also an in-app Ask box with a configured key? The web gets its Ask box through the compose
   route either way.
6. **Does every `accept` persist a fork object?** Recommended: yes, private by default, since programs are
   tiny and lineage is the undo stack. The alternative is an in-memory lineage until `save`. Also:
   **licensing of a model-authored edit** on a CC-BY or all-rights-reserved parent (credits carry; does the
   edit inherit the parent's license, as `fork` does, or is it CC0 as the compose route stamps for web
   composes?).
7. **Strict mode.** Should the debugger offer to *refuse* `Opaque` verbs in a program under debug, so every
   seek is `Exact`, rather than labelling the frame `Approximate`?
8. **Sub-bar stops.** Worth a change to the public engine (`djrust`) so a token breakpoint stops *on its
   step*? v1 stops on the bar line.
9. **Soft-stop with tails.** Needs the engine to stop sequencing while the FX bus rings. Wanted for v2?
10. **A DAP adapter** (VS Code / Zed debugging `.btw`)? It falls out of the verb set; it is a distribution
    decision.
11. **Std library policy.** Who may publish under `std/`, the naming and versioning scheme
    (`std/transitions@2` as a name, CID as the truth), and whether the library is CC0 and public in the
    beats repo (it must be, if the site's public model reads it).
12. **Android with Claude.** There is no MCP server on the phone. Drive it through the site's session
    bridge, or expose the control registry over a LAN/mesh socket from djandroid like `djctl app`?
13. **Deep links with a position** (`…@drop+8`) as part of v1.1 for the web: yes?
14. **Where does the debugger noun's name stop?** `debug` is clear but it also reads as a diagnostics
    flag; `btw debug` keeps it inside the btw noun but then it would be callable from `do btw …` unless
    `program_callable` is per-verb. Per-noun (`debug`) is simpler and recommended.

---

## 12. Prior art, with credit

Public sources only; names and behaviours are from their public documentation or papers, quoted from
memory of those documents and worth a link check before this is published outside the repo.

| Source | What we take |
|---|---|
| **DAW transport: loop braces, locators/markers, cycle mode** (Ableton Live, Logic Pro, Pro Tools, Reaper) | `hold` is a loop brace over a window; section breakpoints are markers; `seek` is a locate that lands on a bar line; the timeline scrub. We add what DAWs lack: the loop re-seeks the *control* state, so a ramp inside the window restarts each lap. |
| **Strudel and TidalCycles** (Alex McLean and the Tidal community; Felix Roos, Alex McLean and contributors for Strudel) | The cycle model and mini-notation btw descends from; the live-coding workflow of evaluate-and-hear-it-next-cycle; Strudel's highlighting of active code, which the firing markers already follow. Neither has a transport that *holds* a position of the program and explains it; that is the gap here. |
| **Sonic Pi** (Sam Aaron) | `cue`/`sync` as time-based synchronisation, and `use_random_seed`: **deterministic randomness** so the same code is the same music, which is the property this design leans on. Its Cue viewer and log are the model for `action: log`. |
| **rr** (Mozilla; Robert O'Callahan et al.) and **omniscient debugging** (Bil Lewis) | Reverse execution as the debugger's strongest tool: `reverse-continue`, `step back`. rr *records*; we do not need to, because the score is a pure function: **time travel for free**. |
| **The Elm debugger and Redux DevTools** | Model = fold of messages through a pure update gives a scrubbable history. Our `Frame` is the analogue of the model at a message index, and `frame_at` is "replay to N" collapsed to a closed form. |
| **gdb / lldb and the Debug Adapter Protocol** (Microsoft; MIT-licensed spec) | Breakpoint vocabulary: temporary (`tbreak`), ignore counts (`hits`), conditions, **logpoints**, `continue`/`next`/`pause`, and the `stackTrace` / `scopes` / `variables` shape that `why` and `inspect` mirror, which is why a DAP adapter is cheap. |
| **Bret Victor, "Inventing on Principle"** (talk) and the **liveness** literature (Steven Tanimoto, "A perspective on the evolution of live programming") | Scrubbing a running program's state and seeing it change immediately; the levels of liveness; `hold` as keeping the program audible while you edit it. |
| **Hazel and live programming with holes** (Omar et al.) | Programs that stay total and meaningful while being edited; our structural edit ops and "a program with an error is refused whole" echo that stance. |
| **aider, Claude Code, and LSP `TextEdit`** | Edits as small, anchored, validated operations instead of whole-file regeneration; a precondition on the old text so stale context is an error. |
| **A/B testing in audio production** | Level-matched comparison: louder is judged better, so the audition matches loudness before it lets you choose (v1.1). |
| **LLM-as-composer pipelines** (the site's compose route: check-and-repair loop with the checker's own diagnostics) | The repair loop, limits, spend cap, public-CC0 rule, which this design extends rather than reinvents. |

---

## Appendix A. Debug-session JSON shapes (summary)

```text
Breakpoint   { id, kind: bar|section|token|control|gen|cond, anchor, cond?, hits?, once, action, enabled, label,
               state: resolved{bar}|orphan{why}|live }
Anchor       bar N | section NAME (+K) | section NAME end | token {scope, deck, role, ordinal} | control KEY
             | gen {scope, n}
Window       { start, len, pre }
Transport    playing | held{window} | paused{at} | stepping{until, then}
Frame        see §2          Inspection  see §5.1          Provenance  list of {kind, span, detail} (§5.3)
Op           replace | insert_before | insert_after | delete {anchor:{line, hash}, text?} | import {name}
Proposal     { id, base_cid, ops, summary, scope, instruction, state: pending|previewing|accepted|rejected,
               result?: {cid, canonical, diagnostics, footprint, plan} }
Footprint    { bars: [{from, to, deck?, section?}], statements: {added, removed, changed}, outside_window: bool }
Lineage      [ { cid, forked_from, instruction, window, by, at } ]
```

## Appendix B. What changes in existing docs when this lands

- `docs/BTW.md`: a "Debugging" section (hold, seek, breakpoints, `why`), the `debug` noun beside the `btw`
  noun, "Not done" loses nothing but gains the three gaps of §7.3, and `import "std/transitions"` joins
  the examples.
- `docs/CONTROL.md` and the registry: the `debug` noun and the `program_callable` and seek-class
  attributes.
- `docs/TESTING.md`: the seek-equals-played suite and the debug UI shots.
- The site: `btwsite/DESIGN.md` Compose section gains edit mode; the MCP tool list gains `debug_*`.
