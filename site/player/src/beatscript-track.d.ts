// Types for <beatscript-track> (beatscript-track.js). The element defines itself on import.

/** What `report` gives: the state the card shows, for a test or a page that watches the player. */
export interface BeatscriptTrackReport {
  playing: boolean;
  /** The section line ("verse", "not playing"). */
  sec: string | null;
  /** The bar line ("bar 2/4"). */
  bar: string | null;
  /** The state line: "loading…", an error, or empty. */
  state: string | null;
  /** The engine's composer (null before the first play). */
  composer: unknown;
  /** The decks whose nets the engine has installed. */
  installed: string[];
  /** The program's CID once it has been checked, else null. */
  cid: string | null;
}

/**
 * A BeatScript program and its player. The program is the child `<script type="text/beatscript">` or the `text`
 * attribute. Attributes: `cid` (the CID the text must have), `compact`, `theme` ("dark" | "light"), `site` (where
 * the plain links point). Fires `play` (detail `{ cid }`) and `stop`.
 */
export class BeatscriptTrack extends HTMLElement {
  readonly playing: boolean;
  /** Play from a click: browsers start audio only from a user gesture. */
  start(): Promise<void>;
  stop(): void;
  toggle(): void;
  readonly report: BeatscriptTrackReport;
  addEventListener(type: 'play', listener: (this: BeatscriptTrack, ev: CustomEvent<{ cid: string }>) => unknown, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: 'stop', listener: (this: BeatscriptTrack, ev: CustomEvent<undefined>) => unknown, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: string, listener: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions): void;
}

declare global {
  interface HTMLElementTagNameMap {
    'beatscript-track': BeatscriptTrack;
  }
}
