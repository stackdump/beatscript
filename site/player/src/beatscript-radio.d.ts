// Types for <beatscript-radio> (beatscript-radio.js): `import '@stackdump/beatscript-player/radio'`.

/** The station's knobs (docs/beatscript/radio.md "Knobs"). */
export type BeatscriptRadioKnob = 'genre' | 'seed' | 'step' | 'forms' | 'tempo' | 'key' | 'length';

export interface BeatscriptRadioSwap {
  /** Date.now() when the incoming item's decks were let go. */
  at: number;
  /** The playlist keys: `s:<N>` a station track, `g:<N>` the segue after track N. */
  from: string | null;
  to: string;
  /** How long before the outgoing item's last bar line the incoming one was let go (ms); null for a Next press. */
  lead_ms: number | null;
  /** How long both engines sounded before the outgoing one faded (ms); null until the incoming one sounds. */
  overlap_ms: number | null;
}

export interface BeatscriptRadioReport {
  playing: boolean;
  /** handoff-model.js: idle | playing | bridging | paused */
  state: 'idle' | 'playing' | 'bridging' | 'paused';
  /** Why it paused: a presence pause (queue-model.js PAUSE) or bridge_timeout; '' when it did not. */
  paused: string;
  item: { kind: 'track' | 'segue'; n: number; key: string; cid: string } | null;
  next: { key: string; state: 'resolving' | 'ready' | 'failed' } | null;
  /** Engines alive: the one playing, any fading out, and the next one primed. */
  engines: number;
  swaps: BeatscriptRadioSwap[];
  station: { cid: string; n: number; knobs: Record<BeatscriptRadioKnob, unknown>; locks: BeatscriptRadioKnob[] } | null;
  rows: { key: string; current: boolean; status: string; title: string }[];
  edited: { cid: string | null } | null;
  state_line: string;
  position: string;
  links: string[];
}

/**
 * `<beatscript-radio station="…" genre="…" seed="…" locks="tempo key" compact autoplay>`: a station and its player. The
 * station's tracks and segues are computed in the page by the engine; nothing is fetched. Events: `play`, `stop`,
 * `trackchange` (detail {kind, n, cid, title}), `bridge` (detail {from}).
 */
export class BeatscriptRadio extends HTMLElement {
  /** Resolves (true) once the station is answered, false when it could not be. */
  ready: Promise<boolean>;
  readonly playing: boolean;
  /** The GainNode every engine's sound goes through (null before the first Play): a page can tap it. */
  readonly output: GainNode | null;
  readonly report: BeatscriptRadioReport;
  start(): Promise<void>;
  stop(): void;
  /** Next: the next item now (a ready one swaps at once, else a bridge plays until it is). */
  skip(): void;
  prev(): Promise<boolean> | null;
  moreLikeThis(): Promise<void>;
  scan(): Promise<void>;
}

/** The knob attributes of an element as a retune ({knob: value}). */
export function knobAttributes(el: Element): Partial<Record<BeatscriptRadioKnob, unknown>>;
/** A `locks` attribute's knobs, in the knobs' order. */
export function lockList(v: string | null): BeatscriptRadioKnob[];

declare global {
  interface HTMLElementTagNameMap {
    'beatscript-radio': BeatscriptRadio;
  }
}
