// Types for <beatscript-studio> (beatscript-studio.js). The config is site/web/embed-config.schema.json's; the tool
// surface is tool-surface.js's (its results are MCP CallToolResults).

export interface StudioProblem {
  line: number;
  col: number;
  severity: 'error' | 'warning';
  message: string;
}

export interface StudioNote {
  what: string;
  status: 'works' | 'degraded' | 'unsupported';
  message: string;
}

export interface ToolResult {
  content: { type: 'text'; text: string }[];
  structuredContent?: unknown;
  isError?: boolean;
}

export interface ToolSurface {
  list(): { name: string; title: string; description: string; inputSchema: object; annotations: object; via: string; available: boolean; why: string | null }[];
  call(name: string, args?: Record<string, unknown>): Promise<ToolResult>;
  handle(message: object): Promise<object | null>;
  asSampleTools(names?: string[]): { name: string; description: string; inputSchema: object; execute(input: unknown): Promise<unknown> }[];
  serve(win: Window, opts?: { origins?: string[] }): () => void;
  /** Stop hearing the room (the element does it when it is taken off the page). */
  close(): void;
}

export interface StudioReport {
  text: string;
  cid: string | null;
  ok: boolean;
  problems: StudioProblem[];
  playing: boolean;
  /** What the host can and cannot do here (resolve's notes), and why. */
  notes: StudioNote[];
  /** The names of the tools available here. */
  tools: string[];
  /** The buttons shown: the toolbar's, and 'dictate' (in the compose panel) and 'speak' (on the status line) where the
   *  dictation and tts capabilities resolved. */
  buttons: string[];
  config: Record<string, unknown>;
  sec: string;
  bar: string;
  state: string;
  composer: string | null;
  installed: string[];
  /** 'engine' when the engine's highlighter colours the editor, 'comments' before it loads. */
  highlighted: 'engine' | 'comments' | null;
  /** The last share link, and whether it reached the clipboard. */
  shared: { url: string; copied: boolean } | null;
  /** The last offline render (audio=render or the WAV export), and how it played. */
  render: { frames: number; bars: number; seconds: number; cid: string; via: 'audio' | 'webaudio' | null } | null;
  /** With a set (or a station's tracks): the item in the editor and every item's title. */
  set: { at: number; titles: string[] } | null;
}

export declare const STARTER: string;

export declare class BeatscriptStudio extends HTMLElement {
  /** The program in the editor. */
  text: string;
  /** The MCP-shaped tools over this studio (null until the engine has loaded). */
  tools: ToolSurface | null;
  readonly report: StudioReport;
  /** Put a program in the editor (one undo step). */
  load(text: string): void;
  check(): object | null;
  fmt(): void;
  share(): Promise<void>;
  /** Open a self-sealing share link (its program is checked against its CID). */
  open(link: string): void;
  generate(genre: string, seed: string | number): void;
  /** Put set item i in the editor; false when that item has only a CID. */
  pick(i: number): boolean;
  /** Play (live), or with Ctrl/Cmd+Enter while playing, put the edited text into the running program. */
  evaluate(): unknown;
  start(): Promise<void>;
  stop(): void;
  renderAndPlay(): Promise<void>;
  /** 'downloads' (the host saved it), 'link' (a download link), or null (not saved). */
  exportBtw(): Promise<string | null>;
  exportWav(): Promise<string | null>;
  addEventListener(type: 'change', listener: (e: CustomEvent<{ text: string }>) => void): void;
  addEventListener(type: 'check', listener: (e: CustomEvent<{ ok: boolean; cid: string | null; problems: StudioProblem[] }>) => void): void;
  addEventListener(type: 'play', listener: (e: CustomEvent<{ cid: string }>) => void): void;
  addEventListener(type: 'stop' | 'ready', listener: (e: CustomEvent) => void): void;
  addEventListener(type: string, listener: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions): void;
}

declare global {
  interface HTMLElementTagNameMap {
    'beatscript-studio': BeatscriptStudio;
  }
}
