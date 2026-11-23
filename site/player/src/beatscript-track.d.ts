// Types for <beatscript-track> (beatscript-track.js) and the offline render (beatscript-render.js). The element defines
// itself on import.

/** What render mode made, once it has rendered (`report.render`). */
export interface BeatscriptRenderInfo {
  frames: number;
  sampleRate: number;
  seconds: number;
  /** The bars rendered: one pass (a looping song once), capped at 256 bars and 600 s. */
  bars: number;
  /** Whether the song loops (`arrange { } loop`): the render is looped when it plays. */
  loops: boolean;
  /** How long the render took, in ms. */
  ms: number;
  /** Where it was rendered: a Worker (the package on the page's origin) or the page's thread (from a CDN). */
  via: 'worker' | 'page';
  /** How it plays: an <audio> on a blob: WAV, or Web Audio (a page whose CSP refuses blob: media). */
  playback: 'audio' | 'webaudio';
  cid: string;
  /** What the render left out or cut ("the song loops: the render is one pass through its sections", a cap). */
  notes: string[];
  /** Frames rendered before the first deck started (the net installing), dropped: the render starts on the downbeat. */
  lead: number;
}

/** What `report` gives: the state the card shows, for a test or a page that watches the player. */
export interface BeatscriptTrackReport {
  playing: boolean;
  mode: 'live' | 'render';
  /** The section line ("verse 1/2", "not playing", "rendering", "paused"). */
  sec: string | null;
  /** The bar line ("bar 2/4"). */
  bar: string | null;
  /** The state line: "loading…", the render's percentage, an error, or empty. */
  state: string | null;
  /** The engine's composer (live mode, while it plays; null otherwise). */
  composer: unknown;
  /** The decks whose nets the engine has installed (live mode). */
  installed: string[];
  /** The program's CID once it has been checked, else null. */
  cid: string | null;
  /** Render mode: the place in the render, in seconds (null before it has rendered). */
  time: number | null;
  /** Render mode: what it rendered, or `{progress}` while it renders, else null. */
  render: BeatscriptRenderInfo | { progress: BeatscriptRenderProgress } | null;
  /** `highlight`: 'comments' (before the engine is loaded), 'engine', or null. */
  highlighted: 'comments' | 'engine' | null;
  /** The last Copy: 'clipboard', 'command' (execCommand), 'selected' (the person copies it), 'failed', or null. */
  copied: 'clipboard' | 'command' | 'selected' | 'failed' | null;
  /** The card's links: the CID's and "Open on beatscript.app". */
  links: { part: 'cid' | 'open'; href: string }[];
}

/**
 * A BeatScript program and its player. The program is the child `<script type="text/beatscript">` or the `text`
 * attribute. Attributes: `mode` ("live", the default: the engine plays as it runs; "render": the first Play renders
 * the whole pass offline, then it plays from memory with a seek bar, pause and loop), `cid` (the CID the text must
 * have), `compact`, `theme` ("dark" | "light"), `site` (where the plain links point), `stored` (the CID links to the
 * site's /p/<CID>; otherwise both links are the self-sealing share link), `highlight` (the source under the card,
 * coloured, with a Copy button).
 * Fires `play` (detail `{ cid }`), `stop`, and in render mode `pause` and `rendered` (detail: frames, seconds, bars,
 * ms, via, playback).
 */
export class BeatscriptTrack extends HTMLElement {
  readonly playing: boolean;
  readonly mode: 'live' | 'render';
  /** Play from a click: browsers start audio only from a user gesture. In render mode the first one renders. */
  start(): Promise<void>;
  /** Silent at once. Live: the next Play starts clean from bar 1. Render: the playback goes back to 0. */
  stop(): void;
  /** Render mode: hold the place. */
  pause(): void;
  toggle(): void;
  /** Render mode: the rendered WAV (16-bit stereo, 44.1 kHz), or null before it has rendered. */
  readonly wav: Blob | null;
  readonly report: BeatscriptTrackReport;
  addEventListener(type: 'play', listener: (this: BeatscriptTrack, ev: CustomEvent<{ cid: string }>) => unknown, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: 'stop' | 'pause', listener: (this: BeatscriptTrack, ev: CustomEvent<undefined>) => unknown, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: 'rendered', listener: (this: BeatscriptTrack, ev: CustomEvent<{ cid: string; seconds: number; frames: number; bars: number; ms: number; via: 'worker' | 'page'; playback: 'audio' | 'webaudio' }>) => unknown, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: string, listener: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions): void;
}

/** onProgress's argument. */
export interface BeatscriptRenderProgress {
  frames: number;
  total: number;
  bars: number;
  totalBars: number;
  fraction: number;
}

/** What renderProgram resolves to. */
export interface BeatscriptRenderResult {
  left: Float32Array;
  right: Float32Array;
  sampleRate: number;
  frames: number;
  bars: number;
  programBars: number;
  bpm: number;
  seconds: number;
  loops: boolean;
  cut: 'bars' | 'seconds' | null;
  notes: string[];
  cid: string;
  sections: { name: string; start: number; bars: number }[];
  lead: number;
  errors: string[];
  ms: number;
}

/**
 * `import { renderProgram } from '@stackdump/beatscript-player/render'`: one pass of `text`, offline, by the engine the
 * element plays (in a page, a Worker or Node). The same text at the same rate is the same PCM every time.
 */
export function renderProgram(opts: {
  /** The compiled wasm/btw_bg.wasm. */
  module: WebAssembly.Module;
  text: string;
  /** Default 44100. */
  sampleRate?: number;
  /** Default { maxBars: 256, maxSeconds: 600 }. */
  limits?: { maxBars: number; maxSeconds: number };
  onProgress?: (p: BeatscriptRenderProgress) => void;
  signal?: AbortSignal;
  /** Give the thread back this often (ms, wall clock); default 50. It changes no sample. */
  yieldMs?: number;
}): Promise<BeatscriptRenderResult>;

declare global {
  interface HTMLElementTagNameMap {
    'beatscript-track': BeatscriptTrack;
  }
}
