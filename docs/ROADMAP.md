# Roadmap

What BeatScript does not do yet and should, found while building something else. An item says what is missing, why it
matters, and where it would go. It is not a promise or an order: the language changes only when a construct earns its
place. The design items of the DJ language are in project-datum (`beatscript-roadmap`, see docs/dj-language.md).

## Mix

Found while building the polish loop (`listen`'s mix analysis, docs/beatscript/polish.md, 2026-11-24). `listen` can say
what is wrong with a mix in these ways, and BeatScript has no line that fixes it directly.

- **A pan on a strip.** `mix { A pan -1 }` is read and refused: the mixer has no pan (docs/beatscript/ext-mix.md "Not
  done"). A part cannot be placed left or right; only a net deck's aux macros pan some bars (`do aux macro=hard-left`).
- **A width control.** Nothing narrows a wide voice or widens a narrow one, so `listen`'s low-end-wide and out-of-phase
  warnings can only say "try another `inst=`". A `width 0..1` on a deck or a part (mid/side gain) would answer them, and a
  "mono under N Hz" switch would answer the commonest one.
- **A limiter and a compressor.** `drive` saturates; there is no limiter on the master, so a quiet master whose true peak is
  already at the ceiling cannot reach a loudness target cleanly. A `master: limit -1db` (a true-peak limiter with a fixed
  release) and a deck `compress` (threshold, ratio) would make `targets` reachable without distortion.
- **EQ on a part.** The EQ is the deck's three knobs (low shelf 250 Hz, mid peak 1 kHz, high shelf 4 kHz), so two parts on one
  deck that clash cannot be told apart by EQ; the fix today is to move one to its own deck. A part param (`eq=low-6`, or a
  `hp=`/`lp=` on the part) and a parametric band (frequency, gain, Q) would fix a clash in place.
- **A duck keyed by any part, ducking a part.** `duck B under A.kick` keys on a kick's (or a bass's) low end and ducks a
  whole deck (the engine's pump). A duck keyed from the compile's own hit times (it knows every part's steps) could follow a
  snare or a vocal, and duck one part of a deck (docs/beatscript/ext-mix.md "Not done").
