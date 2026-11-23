# Perform: DJ moves on the phrase grid

*Beyond Swift.* The classic two-deck moves, each **armed now and landing on the bar / phrase
grid** the decks already keep (a net deck's `tick / 16`, a sample deck's beat grid). Not the
insert tricks of [TRICKS.md](TRICKS.md): a move is a small schedule over existing engine
primitives (the strip EQ, the mute, the crossfader, the insert tricks, the deck's loop, the
seed step, the vocoder swap). No new engine primitive was needed.

The scheduler is **`djhost::perform`** (`crates/djhost/src/perform.rs`): pure, behind the
`MoveHost` trait, which djrust's `Engine` implements there. The desktop window, the control
socket and (next) djandroid's FFI drive the same `Moves`.

| move | at arm | on its line | after | `bars` |
|---|---|---|---|---|
| `bass_swap` | the incoming deck's low EQ cut (−24 dB, the strip's floor) | the lows swapped: incoming back to its level, outgoing cut, in one pass | — | phrase 8/16/32, on the outgoing deck's grid |
| `phrase_sync` | — | `do` = `seed`: `Engine::step_seed(+1)` a bar early, so the seed comes in on the line (as auto-next); `cut`: crossfader to the deck | — | phrase 8/16/32 |
| `filter_exit` | — | the `filter` trick rides a high-pass (0.9) over `bars` from the next bar | the deck muted on the downbeat, the filter released behind it | sweep 1/2/4/8 |
| `echo_out` | — | the `wash` trick on the one: dry gone in a 16th, the room rings | the deck muted `bars` later, the wash off behind it | tail 1/2/4/8 |
| `loop_roll` | — | a 4-beat loop from the bar: a sample deck's on its grid, a net deck's on its steps | halved every `bars` (4 → 2 → 1 → ½), released on the line after the last (the drop). A deck that cannot loop (a sample with no grid) rolls the `stutter` beat repeat 1 → ½ → ¼ → ⅛ | bars per step 1/2/4/8 |
| `vocoder_handoff` | — | `Engine::vocoder_swap` | — | phrase 8/16/32 |

`harmonic` (read-only): both decks' keys (a net deck's project root and scale; a sample deck's
file name) and whether they mix: `same`, `fifth` (±7 semitones, same mode), `relative`
(major / minor a minor third apart), else `clash`.

**Timing.** A line is crossed when the grid deck's beat reaches it, so a move lands within one
service pass (~10 ms in djapp), and everything one pass does lands together. A stopped grid
fires at once, except a seed step, which is refused (it steps on its own deck's bars). A roll
counts its own beats from the service clock, since a looping deck's beat wraps. On a net deck
the halves and the release are `loop` deck actions (`halve`, `exit`), so the action log
carries the roll; the loop's start is the engine's `loop_beats`, a live op. Arming a kind
again on the same deck replaces it; a bass swap and a vocoder handoff are one at a time.
Cancel: waiting moves are dropped (a bass swap gives the lows back); running ones let go (the
filter and the wash open, the roll released).

## Driving it

| from | how |
|---|---|
| the window | **Perform** in the top bar: one button per move (click arms, click again cancels), its deck and `bars` beside it, a light (amber armed, green running), its last word |
| the socket / MCP | `moves` / `djapp_moves`: `action` status (default), arm, cancel, toggle, cancel_all; `kind`, `deck`, `bars`, `do` (seed / cut). Also `state section=moves`. (`perform` was already the deck-action method.) |
| a program | `move bass_swap on B over 8 bars` (or `control: move ...`) calls `moves arm` where its line fires. The offline render (`beatscript render`, `beatscript listen`) arms the same `Moves` on its own engine and services it every block on the render's clock (`djhost::offline_moves`); `filter_exit` and `echo_out` are played by the program's control net there, and a vocoder handoff and a seed step cannot act (the notes say why). A net deck's loop buttons (a roll's loop, its halves, its release) reach the deck one frame after the line, as the booth's service pass is a little after it: on the line's own frame the deck's clock has not reached the line, so the loop would start a beat early and the release would leave the deck out of phase |

## Auto-DJ

The moves can run on their own: [AUTO-DJ.md](AUTO-DJ.md) arms them across a handover every N bars (the
row at the top of the Perform window, config `autodj.*`, the `autodj` method).

## API (for porting)

```rust
use djhost::perform::*;
let mut moves = Moves::new();
let id = moves.arm(&mut engine, MoveSpec { kind: MoveKind::BassSwap, deck: "B".into(), bars: 8, action: PhraseAction::Seed })?;
moves.service(&mut engine, now_secs);   // every service pass
moves.state(&engine);                   // Vec<MoveState>: id, spec, phase (Armed | Running), beats_left, text
moves.cancel(&mut engine, id); moves.cancel_kind(&mut engine, kind, "B"); moves.cancel_all(&mut engine);
moves.to_json(&engine);                 // the socket's status JSON
harmonic_hint(&engine);                 // Hint { a, b, compat }
```

djandroid's `rust/` already depends on `djhost`, so its UniFFI layer can hold a `Moves` beside
its engine and expose arm / cancel / state as above (its engine must be a `MoveHost`: djrust's
`Engine` is one). The Swift app and WASM would do the same through their own bindings.

## Not done

- **Echo out with a delay** (rather than the reverb): the strip's mute and fader come after
  the inserts, so cutting the channel kills the delay's tail, and the EQ floor is −24 dB, not a
  kill. It needs a dry gain on the dub delay (as `wash` has) in djrust's `fx::tricks`.
- **A roll on a net deck across a section line of a program**: the section hands the deck
  another net, and the engine ends a loop when that happens, so the stages after the line
  halve nothing and the roll is let go there (the stutter rolled on through it). A roll that
  sits inside one section, released on the next one's line, is whole.
