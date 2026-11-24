// <beatscript-radio>: a BeatScript radio station and its player, self-contained (the player package, beside <beatscript-track>).
//
//   <script type="module" src="https://cdn.jsdelivr.net/npm/<package>@<version>/beatscript-radio.js"></script>
//   <beatscript-radio></beatscript-radio>                          the techno radio station (station-model.js DEFAULT_STATION)
//   <beatscript-radio station="tempo 124&#10;station {&#10;  from gen@2 house seed=7&#10;}&#10;"></beatscript-radio>
//   <beatscript-radio station="z4EBG9j…"><script type="text/beatscript">…the station's text…</script></beatscript-radio>
//
// A station is a BeatScript program (docs/beatscript/radio.md): position N of a station is always the same track. Its tracks
// and segues are computed in the page by the engine's own station answer (the wasm's beatscript_station, the function that
// answers POST /api/station), so the element needs no server: nothing is fetched from beatscript.app, and the links to it
// are plain links. Sound starts only from a click (autoplay is off unless the `autoplay` attribute asks, and even then the
// browser decides).
//
// Attributes:
//   station    the station's program text, or its CID (the text then comes from a child <script type="text/beatscript">,
//              and must be that CID); none: the techno radio
//   cid        the CID the station's text must have (a text that is not that station is refused)
//   genre seed step forms tempo key length
//              the station's knobs: each one given is a retune of the station before it plays (docs/beatscript/radio.md
//              "Knobs"); forms is a space-separated rotation
//   locks      knobs held fixed (space or comma separated): Scan and "more like this" leave them alone
//   n          the position to start at (default 0)
//   segues     "off": no segue between tracks (default on, as /radio plays)
//   autoplay   start without a click when the browser allows it (default: off; a Claude artifact needs a click)
//   compact    the transport and the title only
//   theme      dark (the default) or light
//   site       where the plain links point (default https://beatscript.app)
//
// What it shows and does (the /radio page's player, PR #4, on the same models):
//   - a transport (previous, play/stop, next), the title, elapsed and total time, the section and bar, the tempo and key,
//     and a level meter;
//   - the playlist: what plays, then the station's next tracks (and the segue after the one playing), each with its
//     status: playing, resolving, ready, bridging, failed;
//   - never dead air (handoff-model.js): what is next is compiled while the current item plays, and primed on an engine
//     of its own (its nets installed, its decks held), so a track change is the held decks let go a moment before the
//     outgoing track's last bar line, the outgoing engine fading out under it. When the next item is not ready in time,
//     a small CC0 bridge loop at the outgoing tempo and key is hot-reloaded into what plays, until it is;
//   - peek: the playing program's text in a drawer. An edit applies on the next bar (a hot reload); "back to station"
//     puts the track back; Copy copies it.
//
// It stores nothing (no cookies, no localStorage) and needs the page to allow WebAssembly ('wasm-unsafe-eval').

import { loadModule } from './beatscript-track.js';
import { createEngine } from './js/engine.js';
import { Driver, EngineSource } from './js/driver.js';
import { Handoff } from './js/handoff-model.js';
import {
  DEFAULT_STATION, KNOBS, initialState, stationRequest, retuneRequest, applyAnswer, stepTo, playOrder, trackStarted,
  scanRequest, fmtValue, maxN, parseStation,
} from './js/station-model.js';
import { fmtTime, nameOf, stationTitle } from './js/playlist-model.js';
import { PresenceTracker, pauseReason, pauseMessage, playBars, passBars } from './js/queue-model.js';
import { renderPlan } from './js/render-model.js';
import { positionModel } from './js/embed-model.js';

const TICK_MS = 25;
const SITE = 'https://beatscript.app';
const CID_RE = /^z[1-9A-HJ-NP-Za-km-z]{20,120}$/;
/** The station tracks the playlist shows after the one playing. */
const AHEAD = 4;
/** The held decks of the next item are let go this many beats before the outgoing item's last bar line. */
const LEAD_BEATS = 0.5;
/** The outgoing engine fades out over this, once the incoming one sounds. */
const FADE_MS = 120;
/** An incoming engine that has not sounded after this is taken as sounding (the outgoing one goes anyway). */
const SWAP_WAIT_MS = 4000;
const SUSPEND_AFTER_MS = 300;

const CSS = `
:host { display: block; max-width: 560px; font: 14px/1.35 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  --bg: #16213e; --lcd: #0d1326; --ink: #eceef6; --dim: #8d95b0; --line: #243055; --btn: #0f3460; --btn-hover: #1a4a7a; --accent: #e94560; --link: #88aaff; --ok: #6bd49a; }
:host([theme="light"]) { --bg: #f6f7fb; --lcd: #e9ecf6; --ink: #1a1a2e; --dim: #5d6585; --line: #d6d9e6; --btn: #dfe6f7; --btn-hover: #c9d5f2; --accent: #c2304a; --link: #2b4fb3; --ok: #1f8a50; }
.box { background: var(--bg); color: var(--ink); border: 1px solid var(--line); border-radius: 4px; overflow: hidden; }
.bar { display: flex; align-items: center; gap: 8px; padding: 4px 10px; border-bottom: 1px solid var(--line); font-size: 11px; letter-spacing: .08em; color: var(--dim); }
.bar .name { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; text-transform: uppercase; }
.led { width: 8px; height: 8px; border-radius: 50%; background: var(--line); }
.led.on { background: var(--ok); } .led.bridge { background: #e5c07b; }
.lcd { margin: 8px 10px 0; padding: 8px 10px; background: var(--lcd); border: 1px solid var(--line); border-radius: 3px; font-variant-numeric: tabular-nums; }
.row { display: flex; gap: 10px; align-items: baseline; min-width: 0; }
.time { font: 600 18px/1.2 ui-monospace, SFMono-Regular, Menlo, monospace; }
.title { font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin: 2px 0; }
.dim { color: var(--dim); font-size: 12px; }
.grow { flex: 1; min-width: 0; }
.meter { height: 4px; margin: 6px 0 0; background: var(--line); border-radius: 2px; overflow: hidden; }
.meter > div { height: 100%; width: 100%; background: var(--accent); transform-origin: left center; transform: scaleX(0); }
.transport { display: flex; gap: 6px; align-items: center; padding: 8px 10px; }
button { border: 0; border-radius: 3px; background: var(--btn); color: var(--ink); cursor: pointer; font: 13px system-ui, sans-serif; padding: 6px 10px; }
button:hover:not(:disabled) { background: var(--btn-hover); }
button:focus-visible { outline: 2px solid var(--link); outline-offset: 2px; }
button:disabled { opacity: .45; cursor: default; }
button.play { min-width: 52px; font-size: 16px; }
button.play.playing { color: var(--accent); }
.state { padding: 0 10px 6px; color: var(--dim); font-size: 12px; }
.state:empty { display: none; }
ol { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); max-height: 220px; overflow: auto; }
li { display: grid; grid-template-columns: 2.4em 1fr auto 3.2em 5.6em; gap: 6px; padding: 4px 10px; font-size: 12px; border-bottom: 1px solid var(--line); font-variant-numeric: tabular-nums; }
li.current { color: var(--accent); font-weight: 600; }
li .t { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
li .s { color: var(--dim); text-align: right; }
li.bridging .s, li.failed .s { color: #e5c07b; }
.tools { display: flex; gap: 6px; padding: 6px 10px; align-items: center; flex-wrap: wrap; }
.tools .grow { color: var(--dim); font-size: 12px; }
.peek { border-top: 1px solid var(--line); padding: 8px 10px; }
.peek[hidden] { display: none; }
textarea { box-sizing: border-box; width: 100%; min-height: 140px; resize: vertical; background: var(--lcd); color: var(--ink); border: 1px solid var(--line); border-radius: 3px;
  font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; padding: 8px; tab-size: 4; }
.foot { padding: 6px 10px; font-size: 12px; color: var(--dim); border-top: 1px solid var(--line); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
a { color: var(--link); }
:host([compact]) .lcd .pos, :host([compact]) ol, :host([compact]) .tools, :host([compact]) .peek, :host([compact]) .foot, :host([compact]) button.peekb { display: none; }
`;

let sheet = null;
function styles() {
  if (!sheet && typeof CSSStyleSheet === 'function') {
    try { sheet = new CSSStyleSheet(); sheet.replaceSync(CSS); } catch { sheet = null; }
  }
  return sheet;
}

const h = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) if (v != null) e.setAttribute(k, v);
  e.append(...kids);
  return e;
};

/** The child <script type="text/beatscript">'s text, its markup indentation taken off; null when there is none. */
function childText(el) {
  const s = el.querySelector('script[type="text/beatscript"], script[type="text/x-beatscript"]');
  if (!s) return null;
  const lines = String(s.textContent || '').replace(/^\n+/, '').replace(/\s+$/, '').split('\n');
  const ind = Math.min(...lines.filter((l) => l.trim()).map((l) => l.match(/^\s*/)[0].length));
  return lines.map((l) => l.slice(Number.isFinite(ind) ? ind : 0)).join('\n') + '\n';
}

/** The knob attributes as a retune ({knob: value}); forms is a list. */
export function knobAttributes(el) {
  const out = {};
  for (const k of KNOBS) {
    const v = el.getAttribute(k);
    if (v == null || v.trim() === '') continue;
    if (k === 'forms') out.forms = v.trim().split(/[\s,]+/);
    else if (k === 'tempo') out.tempo = Number(v);
    else if (k === 'step') out.step = v.trim() === '~' ? '~' : `seed=${v.trim().replace(/^seed=/, '')}`;
    else out[k] = v.trim();
  }
  return out;
}

/** `locks` as knob names, in KNOBS order. */
export function lockList(v) {
  const want = String(v || '').split(/[\s,]+/).filter(Boolean);
  return KNOBS.filter((k) => want.includes(k));
}

export class BeatscriptRadio extends HTMLElement {
  static get observedAttributes() { return ['station', 'cid', 'locks', 'n', 'segues', ...KNOBS]; }

  constructor() {
    super();
    this.ctx = null;
    this.mix = null; // every engine's output goes into this gain, then the analyser, then the speakers
    this.analyser = null;
    this.beatscript = null;
    this.module = null;
    // (not `slot`: an element's own `slot` property is a string attribute)
    this.live = null; // the engine that plays: {engine, src, driver, held, id}
    this.outgoing = []; // engines fading out after a swap
    this.timer = 0;
    this.starting = false;
    this.state = null;
    this.ahead = [];
    this.meta = new Map(); // cid -> {title, seconds, bpm, key, report}
    this.failed = new Set();
    this.swaps = []; // {at, from, to, lead_ms, overlap_ms}: what the tests read
    this.started = false; // a station track has played
    this.auto = 0;
    this.lastAdvanceAt = 0;
    this.seq = 0;
    this.liveSeq = 0;
    this.target = null;
    this.edited = null;
    this.presence = new PresenceTracker({ doc: typeof document !== 'undefined' ? document : null, audio: () => (this.ctx ? this.ctx.state : 'none') });
    this.handoff = new Handoff({
      provider: {
        upcoming: () => this.upcoming(),
        after: (key) => { this.failed.add(key); return this.upcoming(); },
        commit: (d) => this.commit(d),
      },
      sink: {
        play: (p, d) => this.soundPlay(p, d),
        bridge: (text) => this.soundBridge(text),
        stop: () => this.soundStop(),
        discard: (p) => this.discard(p),
      },
      verdict: () => this.verdict(),
      onChange: () => this.paint(),
    });
  }

  // ---- the element ----------------------------------------------------------------------------------------------------------

  connectedCallback() {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    const s = styles();
    if (s) root.adoptedStyleSheets = [s];
    this.site = (this.getAttribute('site') || SITE).replace(/\/+$/, '');
    this.ui = {
      name: h('span', { class: 'name', part: 'name' }, 'BeatScript radio'),
      led: h('span', { class: 'led', 'aria-hidden': 'true' }),
      time: h('span', { class: 'time', part: 'time' }, '0:00'),
      total: h('span', { class: 'dim' }, '/ --:--'),
      flags: h('span', { class: 'dim grow', part: 'flags' }),
      title: h('p', { class: 'title', part: 'title' }, 'Tuning in…'),
      pos: h('span', { class: 'dim', part: 'position' }, 'not playing'),
      tk: h('span', { class: 'dim' }),
      level: h('div'),
      prev: h('button', { type: 'button', 'aria-label': 'Previous track', title: 'Previous track', part: 'prev' }, '⏮'),
      play: h('button', { type: 'button', class: 'play', 'aria-label': 'Play', 'aria-pressed': 'false', part: 'play' }, '▶'),
      next: h('button', { type: 'button', 'aria-label': 'Next track', title: 'Next track', part: 'next' }, '⏭'),
      peekb: h('button', { type: 'button', class: 'peekb', 'aria-expanded': 'false', title: 'Peek at the playing program; it plays on', part: 'peek-button' }, '{ } code'),
      state: h('div', { class: 'state', role: 'status', 'aria-live': 'polite', part: 'state' }),
      rows: h('ol', { part: 'playlist', 'aria-label': 'Playlist: what plays, then the station' }),
      like: h('button', { type: 'button', title: 'The station one seed along, every other knob as it is', part: 'like' }, 'more like this'),
      scan: h('button', { type: 'button', title: 'A notch along a knob that is not locked', part: 'scan' }, 'scan'),
      family: h('span', { class: 'grow', part: 'family' }),
      text: h('textarea', { spellcheck: 'false', autocomplete: 'off', 'aria-label': 'The playing program: edit it and apply to hear it on the next bar', part: 'peek-text' }),
      apply: h('button', { type: 'button', part: 'peek-apply', title: 'Apply on the next bar (Ctrl/Cmd+Enter)' }, 'apply'),
      revert: h('button', { type: 'button', part: 'peek-revert' }, 'back to station'),
      copy: h('button', { type: 'button', part: 'peek-copy' }, 'copy'),
      peekMsg: h('span', { class: 'dim grow' }),
      foot: h('div', { class: 'foot', part: 'links' }),
    };
    const u = this.ui;
    u.peek = h('div', { class: 'peek', part: 'peek', hidden: '' }, u.text, h('div', { class: 'tools' }, u.apply, u.revert, u.copy, u.peekMsg));
    const lcd = h('div', { class: 'lcd', part: 'lcd' },
      h('div', { class: 'row' }, u.time, u.total, u.flags),
      u.title,
      h('div', { class: 'row pos' }, h('span', { class: 'grow' }, u.pos), u.tk),
      h('div', { class: 'meter', 'aria-hidden': 'true', part: 'level' }, u.level));
    root.replaceChildren(h('div', { class: 'box', part: 'box' },
      h('div', { class: 'bar' }, u.led, u.name),
      lcd,
      h('div', { class: 'transport', role: 'group', 'aria-label': 'Transport' }, u.prev, u.play, u.next, h('span', { class: 'grow' }), u.peekb),
      u.state,
      u.rows,
      h('div', { class: 'tools' }, u.like, u.scan, u.family),
      u.peek,
      u.foot));
    u.play.addEventListener('click', () => this.toggle());
    u.next.addEventListener('click', () => this.skip());
    u.prev.addEventListener('click', () => this.prev());
    u.like.addEventListener('click', () => this.moreLikeThis());
    u.scan.addEventListener('click', () => this.scan());
    u.peekb.addEventListener('click', () => this.togglePeek());
    u.apply.addEventListener('click', () => this.applyEdit());
    u.revert.addEventListener('click', () => this.revertEdit());
    u.copy.addEventListener('click', () => this.copyText());
    u.text.addEventListener('keydown', (e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); this.applyEdit(); } });
    this.detachPresence = this.presence.attach(window);
    this.presence.onMediaKey = (a) => {
      if (a === 'play') this.start();
      else if (a === 'pause' || a === 'stop') this.stop();
      else if (a === 'nexttrack') this.skip();
      else if (a === 'previoustrack') this.prev();
    };
    this.ready = this.tune();
    if (this.hasAttribute('autoplay')) this.ready.then(() => this.autoplay());
  }

  disconnectedCallback() {
    this.stop();
    if (this.detachPresence) this.detachPresence();
    clearTimeout(this.suspendTimer);
    if (this.ctx) { try { this.ctx.close(); } catch { /* closed */ } }
    this.ctx = null;
    this.mix = null;
    this.analyser = null;
    this.shadowRoot && this.shadowRoot.replaceChildren();
  }

  attributeChangedCallback(name, old, now) {
    if (!this.shadowRoot || old === now || !this.state) return;
    // a new station or knob: the station is asked for again, and what plays switches to its track N
    this.ready = this.tune({ restart: true });
  }

  say(t) { if (this.ui) this.ui.state.textContent = t || ''; }

  // ---- the station ----------------------------------------------------------------------------------------------------------

  /** The station's text and the CID it must have, from the attributes and the child script. */
  source() {
    const a = this.getAttribute('station');
    const child = childText(this);
    const want = (this.getAttribute('cid') || '').trim();
    const cid = CID_RE.test(want) ? want : null;
    if (want && !cid) return { error: `cid="${want}" is not a CID` };
    if (a != null && CID_RE.test(a.trim())) {
      if (child == null) return { error: `the station ${a.trim()} is a CID: this player fetches nothing, so give its text too, in a child <script type="text/beatscript">` };
      if (cid && cid !== a.trim()) return { error: `station="${a.trim()}" and cid="${cid}" name two stations` };
      return { text: child, cid: a.trim() };
    }
    if (a != null && a.trim()) return { text: a.endsWith('\n') ? a : `${a}\n`, cid };
    if (child != null) return { text: child, cid };
    return { text: DEFAULT_STATION, cid };
  }

  /** The module, once (the same compiled wasm <beatscript-track> uses on this page). */
  async module_() {
    if (!this.beatscript) {
      const { module, beatscript } = await loadModule();
      this.module = module;
      this.beatscript = beatscript;
    }
    if (!this.beatscript.station) throw new Error('this engine has no station answer (beatscript_station): it is older than the radio');
    return this.beatscript;
  }

  /** The station answer, computed here by the engine (the same JSON POST /api/station answers). */
  async call(req) {
    const b = await this.module_();
    return b.station(req);
  }

  /**
   * Read the attributes into a station: its text (checked against `station`'s CID, when it is one), its knobs retuned to
   * the knob attributes, its locks, the position. `restart`: what plays switches to the new station's track N.
   */
  async tune({ restart = false } = {}) {
    const seq = ++this.seq;
    const src = this.source();
    try {
      if (src.error) throw new Error(src.error);
      const p = parseStation(src.text);
      if (!p.ok) throw new Error(`the station does not read: ${(p.errors[0] && `line ${p.errors[0].line}: ${p.errors[0].message}`) || 'not a station'}`);
      const n = Math.max(0, Math.floor(Number(this.getAttribute('n')) || 0));
      let state = initialState({ text: src.text, n, segues: this.getAttribute('segues') !== 'off' });
      if (src.cid) {
        const plain = await this.call(stationRequest(state, { count: 1 }));
        if (seq !== this.seq) return false;
        if (!plain.ok || plain.station.cid !== src.cid) throw new Error(`refused: the station's text is ${plain.ok ? plain.station.cid : 'not a station'}, not ${src.cid}`);
      }
      const changes = knobAttributes(this);
      let req = stationRequest(state, { count: AHEAD + 1 });
      if (Object.keys(changes).length) {
        const r = retuneRequest(state, changes, { count: AHEAD + 1 });
        if (!r.ok) throw new Error(r.error);
        req = r.request;
      }
      req.count = Math.max(1, Math.min(AHEAD + 1, maxN(state.knobs && state.knobs.step) - n + 1));
      const ans = await this.call(req);
      if (seq !== this.seq) return false;
      const applied = applyAnswer(state, ans);
      if (applied.error) throw new Error(applied.error.message);
      state = applied.state;
      // the locks hold from here on (a knob attribute is the station's setting, not a turn)
      state.locks = lockList(this.getAttribute('locks'));
      this.state = state;
      // the station's `#! name:` (the answer's text is canonical: no header lines)
      this.name = nameOf(src.text);
      this.ahead = (ans.tracks || []).filter((t) => t.n > state.n);
      this.started = false;
      this.failed.clear();
      for (const t of [state.now, ...this.ahead]) if (t) this.learn(t.cid, t.text);
      this.say('');
      this.restartOrRefresh(restart);
      this.paint();
      return true;
    } catch (e) {
      if (seq === this.seq) { this.say(String((e && e.message) || e)); this.paint(); }
      return false;
    }
  }

  /** The station at the state's N again (after it moved on): the tracks the playlist shows and the segue after N. */
  async ask({ request = null, restart = false } = {}) {
    if (!this.state) return false;
    const req = request || stationRequest(this.state, { count: AHEAD + 1 });
    req.count = Math.max(1, Math.min(AHEAD + 1, maxN(this.state.knobs && this.state.knobs.step) - req.n + 1));
    const seq = ++this.seq;
    let ans;
    try { ans = await this.call(req); } catch (e) { ans = { ok: false, error: String((e && e.message) || e) }; }
    if (seq !== this.seq) return false;
    const { state, error } = applyAnswer(this.state, ans);
    if (error) { this.say(error.message); return false; }
    this.state = state;
    this.ahead = (ans.tracks || []).filter((t) => t.n > state.n);
    for (const t of [state.now, ...this.ahead]) if (t) this.learn(t.cid, t.text);
    this.restartOrRefresh(restart);
    this.paint();
    return true;
  }

  /** After an answer: what plays switches to track N (a retune, a scan, a step back), else only what is next is looked at. */
  restartOrRefresh(restart) {
    if (restart && this.isPlaying() && this.state.now) {
      this.handoff.drop(this.handoff.next);
      this.handoff.next = null;
      this.handoff.start(this.stationDesc(this.state.now));
    } else this.handoff.refresh();
  }

  /** More like this: the station one seed along, every other knob as it is (a lock on seed says no). */
  async moreLikeThis() {
    this.presence.touch();
    if (!this.state) return;
    if (this.state.locks.includes('seed')) { this.say('The seed is locked: unlock it for more like this.'); return; }
    const r = retuneRequest(this.state, { seed: '+1' });
    if (!r.ok) { this.say(r.error); return; }
    if (await this.ask({ request: r.request, restart: true })) this.say(`More like this: seed ${fmtValue('seed', this.state.knobs.seed)}, still at position ${this.state.n}.`);
  }

  /** Scan: a notch along a knob that is not locked, the same one every time for this station at this position. */
  async scan() {
    this.presence.touch();
    if (!this.state) return;
    const r = scanRequest(this.state);
    if (!r.ok) { this.say(r.error); return; }
    if (await this.ask({ request: r.request, restart: true })) this.say(`Scanned: ${r.pick.knob} a notch ${r.pick.dir === '+1' ? 'up' : 'down'}, to ${fmtValue(r.pick.knob, this.state.knobs[r.pick.knob])}.`);
  }

  learn(cid, text, extra = {}) {
    if (!cid) return;
    const m = this.meta.get(cid) || {};
    if (text && !m.text) m.text = text;
    Object.assign(m, extra);
    this.meta.set(cid, m);
  }

  /** A program's check report (sections, length, tempo), once per CID. */
  reportOf(cid, text) {
    const m = (cid && this.meta.get(cid)) || {};
    if (m.report) return m.report;
    let rep = null;
    try { rep = this.beatscript ? this.beatscript.check(text) : null; } catch { rep = null; }
    if (rep && rep.ok && cid) this.learn(cid, text, { report: rep, seconds: renderPlan(rep).seconds, bpm: rep.tempo });
    return rep;
  }

  stationName() {
    return this.name || (this.state && this.state.knobs ? `${this.state.knobs.genre} radio` : 'BeatScript radio');
  }

  // ---- what plays next (handoff-model.js's provider, station-model.js's play order) --------------------------------------

  order() {
    if (!this.state) return [];
    const cur = this.handoff.current && this.handoff.current.desc;
    return playOrder(this.state, { current: cur, ahead: this.ahead, started: this.started, depth: AHEAD })
      .map((e) => (e.kind === 'segue' ? this.segueDesc(e.segue) : this.stationDesc(e.track)));
  }

  upcoming() {
    return this.order().find((d) => !this.failed.has(d.key)) || null;
  }

  stationDesc(t) {
    const k = (this.state && this.state.knobs) || {};
    return this.desc({ key: `s:${t.n}`, kind: 'track', n: t.n, cid: t.cid, text: t.text, form: t.form, title: stationTitle({ name: this.stationName() }, t), genre: k.genre, tempo: k.tempo, musicKey: k.key });
  }

  segueDesc(s) {
    const k = (this.state && this.state.knobs) || {};
    return this.desc({ key: `g:${s.from}`, kind: 'segue', n: s.from, cid: s.cid, text: s.text, form: 'cross', title: `segue ${s.from} → ${s.to}`, genre: k.genre, tempo: k.tempo, musicKey: k.key });
  }

  /**
   * A descriptor and its prepare(): the program compiled into a run of its own, on an engine of its own whose nets are
   * installed and whose decks are held, so that playing it is letting them go.
   */
  desc(d) {
    d.prepare = async () => {
      await this.audioReady();
      // the swap that made this the next one finishes first: a compile holds the page's thread, and with it the outgoing
      // engine's fade
      await this.settled();
      const report = this.reportOf(d.cid, d.text);
      const run = this.beatscript.newRun();
      const res = run.eval(d.text);
      if (!res || !res.ok) {
        run.free();
        const e = ((res && res.diagnostics) || []).find((x) => x.severity !== 'warning');
        throw new Error(`${d.title} does not play here${e ? `: line ${e.line}: ${e.message}` : ''}`);
      }
      let slot;
      try {
        slot = await this.newSlot();
        const cmds = run.service(null);
        slot.held = cmds.filter((c) => c && c.op === 'play');
        slot.engine.apply(cmds.filter((c) => !(c && c.op === 'play')));
        await slot.engine.idle();
      } catch (e) {
        run.free();
        if (slot) this.dropSlot(slot);
        throw e;
      }
      return { run, res, report, text: d.text, slot };
    };
    return d;
  }

  /** Resolves once no engine is fading out (at most a few seconds). */
  settled() {
    return new Promise((resolve) => {
      const t0 = performance.now();
      const look = () => (this.outgoing.length && performance.now() - t0 < SWAP_WAIT_MS + 1000 ? setTimeout(look, 50) : resolve());
      look();
    });
  }

  /** An item starts: the station's N moves there; the playlist is asked for again. */
  commit(d) {
    this.failed.delete(d.key);
    this.lastAdvanceAt = this.presence.now();
    this.edited = null;
    if (d.kind === 'track') {
      this.started = true;
      if (d.n !== this.state.n) {
        this.state = trackStarted(this.state, d.n);
        this.ask();
      }
    }
    if (this.ui && !this.ui.peek.hidden) this.showPeek();
    this.dispatchEvent(new CustomEvent('trackchange', { detail: { kind: d.kind, n: d.n, cid: d.cid, title: d.title } }));
    this.paint();
  }

  verdict() {
    const interacted = this.presence.lastInteraction > this.lastAdvanceAt;
    if (interacted) this.auto = 0;
    const why = pauseReason(this.presence.snapshot(), { auto: this.auto, interactedSinceAdvance: interacted });
    if (!why) this.auto++;
    return why;
  }

  // ---- the sound: one engine per item, the next one primed while the current plays ----------------------------------------

  /** The context and the mix bus, made from a click. */
  audio() {
    if (!this.ctx) this.ctx = new AudioContext({ latencyHint: 'playback' });
    if (!this.mix) {
      this.analyser = this.ctx.createAnalyser();
      this.analyser.fftSize = 1024;
      this.mix = this.ctx.createGain();
      this.mix.connect(this.analyser);
      this.analyser.connect(this.ctx.destination);
    }
    clearTimeout(this.suspendTimer);
    return this.ctx.resume();
  }

  /** The output every engine goes through (a GainNode), for a page that wants to tap it; null before a Play. */
  get output() { return this.mix; }

  async audioReady() {
    await this.module_();
    if (!this.ctx || !this.mix) throw new Error('press play first');
  }

  async newSlot() {
    const engine = await createEngine({ context: this.ctx, module: this.module, destination: this.mix });
    const slot = { id: ++this.liveSeq, engine, src: new EngineSource(engine), driver: new Driver(this.beatscript), held: [], sounding: false };
    engine.on('error', (e) => { if (e && e.message) this.say(e.message); });
    // the worklet's own report keeps the runtime serviced while timers are throttled (a hidden tab)
    engine.on('grid', () => { if (slot === this.live) this.tick(); });
    return slot;
  }

  dropSlot(slot) {
    if (!slot) return;
    try { slot.driver.stopSound(); } catch { /* stopped */ }
    try { slot.engine.dispose(); } catch { /* gone */ }
  }

  discard(p) {
    if (!p) return;
    try { if (p.run) p.run.free(); } catch { /* freed */ }
    p.run = null;
    this.dropSlot(p.slot);
  }

  soundPlay(p, d) {
    const from = this.live;
    const slot = p.slot;
    slot.driver.adopt(p.run, p.text, p.res, slot.src);
    p.run = null; // the driver has it now
    if (slot.held.length) slot.engine.apply(slot.held);
    slot.held = [];
    slot.startedAt = performance.now();
    this.live = slot;
    if (from) {
      from.fadeAt = 0;
      from.swapAt = performance.now();
      this.outgoing.push(from);
      this.swaps.push({ at: Date.now(), from: from.key || null, to: d.key, lead_ms: from.leadMs != null ? Math.round(from.leadMs) : null, overlap_ms: null });
    }
    slot.key = d.key;
    const rep = p.report || {};
    this.target = playBars(null, rep) ?? passBars(rep);
    this.pos = {};
    this.say('');
    this.paint();
  }

  /** A bridge: hot-reloaded into the running program (it lands on the next bar line). */
  soundBridge(text) {
    if (!this.live) return;
    const res = this.live.driver.evaluate(text, this.live.src);
    if (!res || !res.ok) console.warn('beatscript radio: the bridge did not compile', res && res.diagnostics);
    this.target = null;
    this.dispatchEvent(new CustomEvent('bridge', { detail: { from: this.live.key } }));
  }

  soundStop() {
    for (const o of this.outgoing) this.dropSlot(o);
    this.outgoing = [];
    this.dropSlot(this.live);
    this.live = null;
    this.pos = {};
  }

  /** Outgoing engines: serviced until the incoming one sounds, then faded out and let go. */
  serviceOutgoing(now) {
    const inc = this.live;
    const sounding = inc && (inc.sounding || (inc.sounding = !!inc.engine.grid()) || now - inc.startedAt > SWAP_WAIT_MS);
    for (const o of [...this.outgoing]) {
      try { o.driver.tick(); } catch { /* going */ }
      if (!sounding) continue;
      if (!o.fadeAt) {
        o.fadeAt = now;
        o.engine.setLevel(0);
        const s = this.swaps.find((x) => x.to === (inc && inc.key) && x.overlap_ms == null);
        if (s) s.overlap_ms = Math.round(now - o.swapAt);
      } else if (now - o.fadeAt > FADE_MS) {
        this.dropSlot(o);
        this.outgoing.splice(this.outgoing.indexOf(o), 1);
      }
    }
  }

  // ---- the transport ----------------------------------------------------------------------------------------------------------

  isPlaying() { return !!this.handoff.current; }

  get playing() { return this.isPlaying() && !this.handoff.paused; }

  toggle() {
    if (this.starting) return;
    if (this.playing) this.stop();
    else this.start();
  }

  async autoplay() {
    try {
      if (!this.ctx) this.ctx = new AudioContext({ latencyHint: 'playback' });
      if (this.ctx.state !== 'running') await Promise.race([this.ctx.resume(), new Promise((r) => setTimeout(r, 300))]);
      if (this.ctx.state === 'running') return this.start();
      this.say('Press play: this page needs a click before it makes sound.');
    } catch { /* a click starts it */ }
    return null;
  }

  async start(desc = null) {
    this.presence.touch();
    if (this.starting) return;
    this.starting = true;
    const resumed = this.audio();
    this.say('Loading the player…');
    try {
      await resumed;
      await this.ready;
      await this.audioReady();
      if (!this.state) throw new Error(this.ui.state.textContent || 'the station is not answered');
      this.auto = 0;
      this.lastAdvanceAt = this.presence.now();
      clearInterval(this.timer);
      this.timer = setInterval(() => this.tick(), TICK_MS);
      const nx = this.handoff.next;
      if (!desc && this.handoff.paused && nx && nx.state === 'ready') {
        // "Still listening?": the item the pause stopped before is ready (primed), so it plays as it is
        this.handoff.paused = '';
        this.handoff.play(nx.desc, nx.prepared);
      } else {
        const d = desc || this.resumeDesc();
        if (!d) { this.say('Waiting for the station…'); return; }
        // a next item primed for what played before is let go (start prepares its own)
        if (nx && nx.desc.key !== d.key) { this.handoff.drop(nx); this.handoff.next = null; }
        await this.handoff.start(d);
      }
      this.say('');
      this.dispatchEvent(new CustomEvent('play', { detail: { station: this.state.cid, n: this.state.n } }));
    } catch (e) {
      this.say(String((e && e.message) || e));
    } finally {
      this.starting = false;
      this.paint();
    }
  }

  /** What Play plays: the item a pause stopped before, else the station's track N. */
  resumeDesc() {
    const nx = this.handoff.next;
    if (this.handoff.paused && nx && nx.state !== 'failed') return nx.desc;
    return this.state && this.state.now ? this.stationDesc(this.state.now) : null;
  }

  /** Next: what is next now (a ready one swaps at once; one still resolving is bridged to). */
  skip() {
    this.presence.touch();
    if (!this.isPlaying()) return;
    if (this.live) this.live.leadMs = null;
    this.handoff.boundary();
  }

  prev() {
    this.presence.touch();
    if (!this.state) return null;
    const cur = this.handoff.current && this.handoff.current.desc;
    const n = cur && cur.kind === 'track' ? cur.n - 1 : this.state.n - 1;
    if (n < 0) return null;
    this.state = stepTo(this.state, n);
    return this.ask({ restart: true });
  }

  stop() {
    clearInterval(this.timer);
    this.timer = 0;
    const was = this.isPlaying();
    this.handoff.stop();
    // what is next is let go too: its engine holds its decks, and a later Play prepares afresh
    const nx = this.handoff.next;
    if (nx) { if (nx.prepared) this.discard(nx.prepared); this.handoff.next = null; }
    if (this.ctx && this.ctx.state === 'running') {
      clearTimeout(this.suspendTimer);
      const ctx = this.ctx;
      this.suspendTimer = setTimeout(() => { if (!this.isPlaying() && ctx.state === 'running') ctx.suspend().catch(() => {}); }, SUSPEND_AFTER_MS);
    }
    if (was) this.dispatchEvent(new CustomEvent('stop'));
    this.paint();
  }

  tick() {
    const now = performance.now();
    if (this.outgoing.length) this.serviceOutgoing(now);
    if (!this.isPlaying() || !this.live) { this.level(); return; }
    const f = this.live.driver.tick();
    if (!f) return;
    const st = f.status || {};
    const g = f.grid || {};
    const bar = Math.max(0, st.bar || 0);
    const inBar = Number.isFinite(g.beat) && g.beat > 0 ? g.beat % 4 : 0;
    const c = this.handoff.current;
    const bpm = g.bpm || (c && c.desc.tempo) || 120;
    const plan = c && c.prepared && c.prepared.report ? renderPlan(c.prepared.report) : null;
    this.pos = { status: st, sections: (c && c.prepared && c.prepared.report && c.prepared.report.sections) || [], elapsed: Math.max(0, (bar * 4 + inBar) * 60 / bpm), total: this.handoff.bridging ? null : plan && plan.seconds, bpm };
    this.handoff.frame({ bar });
    if (!this.handoff.bridging && this.handoff.current) {
      const left = this.target != null ? this.target * 4 - (bar * 4 + inBar) : Infinity;
      const nx = this.handoff.next;
      // the next item's decks are let go a moment before this one's last bar line (when it is ready), so that there is no
      // silence between them; a next item that is not ready is bridged at the line itself
      const early = nx && nx.state === 'ready' && left <= LEAD_BEATS;
      if (st.ended || left <= 0 || early) {
        this.live.leadMs = Number.isFinite(left) && left > 0 ? (left * 60000) / bpm : 0;
        this.handoff.boundary();
      }
    }
    this.paintPos();
  }

  level() {
    if (!this.analyser || !this.ui) return 0;
    const a = this.levelBuf || (this.levelBuf = new Float32Array(this.analyser.fftSize));
    this.analyser.getFloatTimeDomainData(a);
    let s = 0;
    for (let i = 0; i < a.length; i++) s += a[i] * a[i];
    const rms = Math.sqrt(s / a.length);
    this.ui.level.style.transform = `scaleX(${Math.min(1, rms * 4).toFixed(3)})`;
    return rms;
  }

  // ---- peek -------------------------------------------------------------------------------------------------------------------

  togglePeek() {
    const open = this.ui.peek.hidden;
    this.ui.peek.hidden = !open;
    this.ui.peekb.setAttribute('aria-expanded', String(open));
    if (open) this.showPeek();
  }

  showPeek() {
    const c = this.handoff.current;
    const text = this.edited ? this.edited.text : c ? c.desc.text : (this.state && this.state.now ? this.state.now.text : '');
    this.ui.text.value = text || '';
    this.ui.peekMsg.textContent = this.edited ? `edited (${(this.edited.cid || '').slice(0, 10)}…)` : '';
  }

  applyEdit() {
    const c = this.handoff.current;
    if (!c || !this.live) { this.ui.peekMsg.textContent = 'press play first: an edit applies to what plays'; return; }
    const text = this.ui.text.value;
    const res = this.live.driver.evaluate(text, this.live.src);
    if (!res || !res.ok) {
      const e = ((res && res.diagnostics) || []).find((x) => x.severity !== 'warning');
      this.ui.peekMsg.textContent = e ? `line ${e.line}: ${e.message} (the track plays on)` : 'it does not compile (the track plays on)';
      return;
    }
    this.edited = { original: c.desc.text, text, cid: res.cid || null };
    this.ui.peekMsg.textContent = `edited: on the next bar (${String(res.cid || '').slice(0, 10)}…)`;
    this.paint();
  }

  revertEdit() {
    const c = this.handoff.current;
    if (!c || !this.live || !this.edited) { this.showPeek(); return; }
    this.live.driver.evaluate(c.desc.text, this.live.src);
    this.edited = null;
    this.showPeek();
    this.ui.peekMsg.textContent = 'back to the station: on the next bar';
    this.paint();
  }

  async copyText() {
    const t = this.ui.text.value;
    try {
      if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('no clipboard');
      await navigator.clipboard.writeText(t);
      this.ui.peekMsg.textContent = 'copied';
    } catch {
      // a frame without clipboard-write: the text is selected for the reader to copy
      this.ui.text.focus();
      this.ui.text.select();
      this.ui.peekMsg.textContent = 'selected: press Ctrl+C';
    }
  }

  // ---- painting ---------------------------------------------------------------------------------------------------------------

  rows() {
    const rows = [];
    const c = this.handoff.current;
    const nx = this.handoff.next;
    const sec = (cid) => (this.meta.get(cid) || {}).seconds;
    if (c) rows.push({ key: c.desc.key, current: true, n: c.desc.n, title: c.desc.title, form: c.desc.form, bpm: c.desc.tempo, seconds: sec(c.desc.cid), status: this.handoff.bridging ? 'bridging' : this.edited ? 'edited' : 'playing' });
    for (const d of this.order()) {
      if (c && d.key === c.desc.key) continue;
      const status = this.failed.has(d.key) ? 'failed' : nx && nx.desc.key === d.key ? nx.state : '';
      rows.push({ key: d.key, current: false, n: d.n, title: d.title, form: d.form, bpm: d.tempo, seconds: sec(d.cid), status });
    }
    return rows;
  }

  paint() {
    if (!this.ui) return;
    const u = this.ui;
    const playing = this.playing;
    u.play.textContent = playing ? '■' : '▶';
    u.play.classList.toggle('playing', playing);
    u.play.setAttribute('aria-pressed', String(playing));
    u.play.setAttribute('aria-label', playing ? 'Stop' : 'Play');
    u.next.disabled = !playing;
    u.led.className = `led${this.handoff.bridging ? ' bridge' : playing ? ' on' : ''}`;
    const k = this.state && this.state.knobs;
    u.name.textContent = this.state ? `${this.stationName()} · #${this.state.n}` : 'BeatScript radio';
    const c = this.handoff.current;
    u.title.textContent = this.handoff.bridging ? `bridging · ${c ? c.desc.title : ''}` : c ? c.desc.title : this.state && this.state.now ? stationTitle({ name: this.stationName() }, this.state.now) : 'Tuning in…';
    u.flags.textContent = this.handoff.paused === 'bridge_timeout' ? 'stopped: nothing was ready' : this.handoff.paused ? 'paused' : this.handoff.bridging ? 'BRIDGE' : playing ? 'LIVE' : '';
    if (this.handoff.paused && this.handoff.paused !== 'bridge_timeout') this.say(pauseMessage(this.handoff.paused));
    if (this.handoff.paused === 'bridge_timeout') this.say('Nothing was ready to play for ten minutes, so the radio stopped. Press play to try again.');
    u.tk.textContent = k ? `${k.tempo ?? ''} bpm · ${k.key ?? ''}` : '';
    u.family.textContent = k ? `${k.genre} · seed ${fmtValue('seed', k.seed)} · step ${k.step} · ${k.forms.join(' ')}${this.state.locks.length ? ` · locked ${this.state.locks.join(', ')}` : ''}` : '';
    u.like.disabled = !this.state;
    u.scan.disabled = !this.state;
    const rows = this.rows();
    u.rows.replaceChildren(...rows.map((r) => h('li', { class: `${r.current ? 'current ' : ''}${r.status || ''}`, 'data-key': r.key, part: 'row' },
      h('span', {}, r.n != null ? `#${r.n}` : ''),
      h('span', { class: 't', title: r.title }, r.title),
      h('span', { class: 's' }, r.form && r.form !== 'seeded' ? r.form : ''),
      h('span', { class: 's' }, fmtTime(r.seconds)),
      h('span', { class: 's' }, r.status || ''))));
    if (this.state) {
      const href = `${this.site}/radio?s=${encodeURIComponent(this.state.text)}&n=${this.state.n}`;
      u.foot.replaceChildren(
        h('span', { title: this.state.cid || '' }, `station ${(this.state.cid || '').slice(0, 10)}… · CC0-1.0 · `),
        h('a', { href, target: '_blank', rel: 'noopener', part: 'open' }, 'Open on beatscript.app'));
    }
    this.paintPos();
  }

  paintPos() {
    if (!this.ui) return;
    const p = this.pos || {};
    const m = positionModel(p.status || null, p.sections || [], this.playing && !!p.status);
    this.ui.pos.textContent = this.handoff.bridging ? 'bridge loop: the next track comes in on a bar line' : `${m.sec}${m.bar ? ` · ${m.bar}` : ''}`;
    this.ui.time.textContent = fmtTime(p.elapsed || 0);
    this.ui.total.textContent = `/ ${fmtTime(p.total)}`;
    this.level();
  }

  /** What the tests read: what plays, what is next, the swaps (with how long both engines sounded), the station. */
  get report() {
    const c = this.handoff.current;
    const nx = this.handoff.next;
    return {
      playing: this.playing,
      state: this.handoff.state,
      paused: this.handoff.paused || '',
      item: c ? { kind: c.desc.kind, n: c.desc.n, key: c.desc.key, cid: c.desc.cid } : null,
      next: nx ? { key: nx.desc.key, state: nx.state } : null,
      engines: (this.live ? 1 : 0) + this.outgoing.length + (nx && nx.prepared ? 1 : 0),
      swaps: this.swaps.map((s) => ({ ...s })),
      station: this.state ? { cid: this.state.cid, n: this.state.n, knobs: { ...this.state.knobs }, locks: [...this.state.locks] } : null,
      rows: this.ui ? this.rows().map((r) => ({ key: r.key, current: r.current, status: r.status, title: r.title })) : [],
      edited: this.edited ? { cid: this.edited.cid } : null,
      state_line: this.ui ? this.ui.state.textContent : '',
      position: this.ui ? this.ui.pos.textContent : '',
      links: this.ui ? [...this.ui.foot.querySelectorAll('a')].map((a) => a.getAttribute('href')) : [],
    };
  }
}

if (typeof customElements !== 'undefined' && !customElements.get('beatscript-radio')) customElements.define('beatscript-radio', BeatscriptRadio);
