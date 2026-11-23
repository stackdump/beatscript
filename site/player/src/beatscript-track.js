// <beatscript-track>: a BeatScript program and its player, self-contained (project-datum 6669).
//
//   <script type="module" src="https://cdn.jsdelivr.net/npm/<package>@<version>/beatscript-track.js"></script>
//   <beatscript-track>
//     <script type="text/beatscript">
//   tempo 120
//   deck A: kick x ~ x ~
//     </script>
//   </beatscript-track>
//
// or `text="…"` instead of the child script, or the files beside the page (`./beatscript-track.js`). Attributes:
// mode (live | render), compact, theme (dark | light), cid (optional: the CID the text must have; a text that is not it
// is refused), site (where the plain links point, default https://beatscript.app), stored (the program is stored on the
// site: its CID links to /p/<CID>), highlight (the program's text under the card, coloured as the editor colours it,
// with a Copy button: comments at once, everything once the engine's btw_highlight is loaded, which fetches the module
// the player uses, once).
//
// Two ways to play, one engine:
//   live    (the default) the engine runs as it plays: an AudioWorklet, the runtime serviced every 25 ms.
//   render  the first Play renders one pass of the arrangement offline, faster than real time (beatscript-render.js:
//           the same engine and worklet code on a clock of its own; in a Worker when the package is on the page's
//           origin, else on the page's thread in slices), shows how far it has got, then plays the result from memory
//           with a seek bar, pause and loop: through an <audio> element on a blob: WAV where the page allows blob:
//           media, else through Web Audio (an artifact's CSP, default-src 'self', refuses blob: media). A later Play
//           plays the same render. A looping song (`arrange { } loop`) renders one pass and loops it. The same program
//           renders to the same PCM every time.
//
// Everything the player needs is in the package: the engine (the same wasm the editor runs, its kits, voices and the
// sing phoneme tables built in, the module inlined as base64 so it is never fetched: a page whose connect-src is 'self'
// can still load it from a CDN), the worklet, the composer, this element. Nothing is fetched from beatscript.app: the
// CID and "Open on beatscript.app" are plain links, both the self-sealing share link (the program is in the link, so it
// opens whether or not the site has stored it; with `stored`, the CID links to /p/<CID>). Sound starts only from a click,
// and Stop is silent at once. The styles are a constructed stylesheet in the shadow root (no <style>, no style
// attribute), so a page whose style-src is 'self' shows them.
//
// The page's CSP must allow WebAssembly: script-src needs 'wasm-unsafe-eval' (or 'unsafe-eval'); without it no wasm
// engine can run, and the element says so.

import { makeFacade } from './js/wasm.js';
import { createEngine } from './js/engine.js';
import { positionModel } from './js/embed-model.js';
import { normClass, plainSpans } from './js/highlight.js';
import { renderPlan, statusAt, clock, wav, RENDER_LIMITS } from './render-model.js';

const TICK_MS = 25;
const SITE = 'https://beatscript.app';
const CID_RE = /^z[1-9A-HJ-NP-Za-km-z]{20,120}$/;
/** The render's sample rate (btwd render's default). */
const RENDER_RATE = 44100;
/** After a stop, the context is suspended this much later (the output is already cut: this only lets the device sleep). */
const SUSPEND_AFTER_MS = 300;
/** How long a blob: WAV may take to load before Web Audio plays it instead. */
const BLOB_PROBE_MS = 4000;

const CSS = `
:host { display: block; max-width: 480px; font: 14px/1.35 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  --bg: #16213e; --ink: #eceef6; --dim: #8d95b0; --line: #243055; --btn: #0f3460; --btn-hover: #1a4a7a; --accent: #e94560; --link: #88aaff; }
:host([theme="light"]) { --bg: #f6f7fb; --ink: #1a1a2e; --dim: #5d6585; --line: #d6d9e6; --btn: #dfe6f7; --btn-hover: #c9d5f2; --accent: #c2304a; --link: #2b4fb3; }
.card { display: grid; grid-template-columns: auto 1fr; gap: 4px 14px; padding: 14px 16px; background: var(--bg); color: var(--ink);
  border: 1px solid var(--line); border-radius: 4px; min-height: 152px; box-sizing: border-box; align-items: center; }
:host([compact]) .card { min-height: 80px; padding: 8px 12px; }
button.play { grid-row: 1 / span 2; width: 64px; height: 64px; border-radius: 50%; border: 0; background: var(--btn); color: var(--ink); cursor: pointer; font-size: 24px; }
:host([compact]) button.play { width: 44px; height: 44px; font-size: 16px; }
button:hover:not(:disabled) { background: var(--btn-hover); }
button:focus-visible { outline: 2px solid var(--link); outline-offset: 2px; }
button:disabled { opacity: .45; cursor: default; }
button.play.playing { color: var(--accent); }
.body { min-width: 0; }
.name { font-size: 16px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin: 0; }
p { margin: 2px 0 0; }
.pos { font-variant-numeric: tabular-nums; }
.dim, .state { color: var(--dim); }
.links { font-size: 12px; color: var(--dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
a { color: var(--link); }
.state:empty { display: none; }
.transport { display: flex; align-items: center; gap: 8px; margin-top: 4px; font-size: 12px; color: var(--dim); font-variant-numeric: tabular-nums; }
.transport[hidden] { display: none; }
.transport input[type="range"] { flex: 1; min-width: 60px; accent-color: var(--accent); margin: 0; }
.transport label { display: inline-flex; align-items: center; gap: 3px; cursor: pointer; white-space: nowrap; }
.transport input[type="checkbox"] { margin: 0; accent-color: var(--accent); }
.src { margin-top: 6px; background: var(--bg); color: var(--ink); border: 1px solid var(--line); border-radius: 4px; position: relative; }
.src[hidden] { display: none; }
.src pre { margin: 0; padding: 10px 12px; overflow: auto; max-height: 360px; font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre; tab-size: 4; }
.src button.copy { position: absolute; top: 6px; right: 6px; border: 1px solid var(--line); border-radius: 3px; background: var(--btn); color: var(--ink); font: 11px system-ui, sans-serif; padding: 2px 8px; cursor: pointer; }
/* the editor's colours (site/web/css/btw.css), and their light-theme counterparts */
.c-comment { color: #6c786c; } .c-keyword { color: #c678dd; } .c-role { color: #61afef; } .c-action { color: #e5c07b; }
.c-deck { color: #e06c75; } .c-number { color: #d19a66; } .c-step, .c-note { color: #98c379; } .c-rest { color: #7a7a86; }
.c-generator, .c-def { color: #56b6c2; } .c-param { color: #d19a66; } .c-op, .c-punct { color: #abb2bf; } .c-string { color: #98c379; }
.c-error { color: #e05050; } .c-ident, .c-section { color: #e5c07b; } .c-word { color: var(--ink); }
:host([theme="light"]) .c-comment { color: #8a8f98; } :host([theme="light"]) .c-keyword { color: #a626a4; } :host([theme="light"]) .c-role { color: #4078f2; }
:host([theme="light"]) .c-action, :host([theme="light"]) .c-ident, :host([theme="light"]) .c-section { color: #986801; }
:host([theme="light"]) .c-deck { color: #e45649; } :host([theme="light"]) .c-number, :host([theme="light"]) .c-param { color: #b76b01; }
:host([theme="light"]) .c-step, :host([theme="light"]) .c-note, :host([theme="light"]) .c-string { color: #50a14f; } :host([theme="light"]) .c-rest { color: #9a9aa6; }
:host([theme="light"]) .c-generator, :host([theme="light"]) .c-def { color: #0184bc; } :host([theme="light"]) .c-op, :host([theme="light"]) .c-punct { color: #696c77; }
:host([theme="light"]) .c-error { color: #ca1243; }
`;

let sheet = null;
function styles() {
  if (!sheet && typeof CSSStyleSheet === 'function') {
    try { sheet = new CSSStyleSheet(); sheet.replaceSync(CSS); } catch { sheet = null; }
  }
  return sheet;
}

/** The module, once per page: compiled from the inlined bytes (a dynamic import, so nothing loads before a click). */
let shared = null;
function loadModule() {
  if (!shared) {
    shared = (async () => {
      const [{ WASM_BASE64 }, glue] = await Promise.all([import('./wasm/btw_bg.wasm.b64.js'), import('./wasm/btw.js')]);
      const bin = atob(WASM_BASE64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      let module;
      try {
        module = await WebAssembly.compile(bytes);
      } catch (e) {
        throw new Error(/unsafe-eval|Content Security/i.test(String(e)) ? 'this page does not allow WebAssembly (its CSP needs \'wasm-unsafe-eval\'), so it cannot play here' : String(e.message || e));
      }
      glue.initSync({ module });
      return { module, btw: makeFacade(glue) };
    })();
    shared.catch(() => { shared = null; });
  }
  return shared;
}

/** The program text: the `text` attribute, else the first child <script type="text/beatscript">, else the element's text. */
function sourceOf(el) {
  const a = el.getAttribute('text');
  if (a != null) return a;
  const s = el.querySelector('script[type="text/beatscript"], script[type="text/x-beatscript"]');
  const t = s ? s.textContent : el.textContent;
  // the indentation of the markup is not part of the program
  const lines = String(t || '').replace(/^\n+/, '').replace(/\s+$/, '').split('\n');
  const ind = Math.min(...lines.filter((l) => l.trim()).map((l) => l.match(/^\s*/)[0].length));
  return lines.map((l) => l.slice(Number.isFinite(ind) ? ind : 0)).join('\n') + '\n';
}

/** `#! name:`, else ''. */
function nameOf(text) {
  const m = /^#!\s*name:\s*(.+)$/m.exec(text);
  return m ? m[1].trim() : '';
}

/**
 * `code`'s children: `text` with `spans` ([[start16, end16, class], ...], as btw_highlight and plainSpans give them) as
 * <span class="c-...">, built as nodes (no innerHTML). As highlight.js spansToHtml: sorted, the earlier span wins an
 * overlap, an unknown class is plain text.
 */
function paintCode(code, text, spans) {
  const list = [];
  for (const s of spans || []) {
    if (!s || s.length < 3) continue;
    const a = Math.max(0, s[0] | 0);
    const b = Math.min(text.length, s[1] | 0);
    if (b > a) list.push([a, b, normClass(s[2])]);
  }
  list.sort((x, y) => x[0] - y[0] || y[1] - x[1]);
  const kids = [];
  let cur = 0;
  for (const [a0, b, cls] of list) {
    const a = Math.max(a0, cur);
    if (b <= a) continue;
    if (a > cur) kids.push(text.slice(cur, a));
    const sp = document.createElement('span');
    sp.className = `c-${cls}`;
    sp.textContent = text.slice(a, b);
    kids.push(sp);
    cur = b;
  }
  kids.push(text.slice(cur));
  code.replaceChildren(...kids);
}

const h = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  e.append(...kids);
  return e;
};

// ---- the render, in a Worker when the browser allows one from where this file is, else here

let workerState = null; // null untried | {worker} | false (none: render here)
function renderWorker() {
  if (workerState !== null) return Promise.resolve(workerState);
  workerState = (async () => {
    try {
      const url = new URL('./render-worker.js', import.meta.url);
      // a Worker's script must be on the page's origin: from a CDN the constructor throws (or the CSP refuses it)
      if (typeof Worker !== 'function' || typeof location === 'undefined' || url.origin !== location.origin) return false;
      const w = new Worker(url, { type: 'module' });
      await new Promise((resolve, reject) => {
        const t = setTimeout(() => reject(new Error('the render worker did not start')), 10000);
        w.onmessage = (e) => { if (e.data && e.data.type === 'ready') { clearTimeout(t); resolve(); } };
        w.onerror = (e) => { clearTimeout(t); reject(new Error((e && e.message) || 'the render worker failed')); };
      });
      return { worker: w, next: 1, jobs: new Map() };
    } catch {
      return false;
    }
  })().then((s) => {
    workerState = s;
    if (s) {
      s.worker.onmessage = (e) => {
        const m = e.data;
        const job = m && s.jobs.get(m.id);
        if (!job) return;
        if (m.type === 'progress') job.onProgress && job.onProgress(m);
        else { s.jobs.delete(m.id); if (m.type === 'done') job.resolve(m.result); else job.reject(new Error(m.error)); }
      };
    }
    return s;
  });
  return workerState;
}

/** Render `text` once: {result, via} (via 'worker' | 'page'). */
async function renderOnce(module, text, onProgress, signal) {
  const ws = await renderWorker();
  if (ws) {
    const id = ws.next++;
    const p = new Promise((resolve, reject) => ws.jobs.set(id, { resolve, reject, onProgress }));
    ws.worker.postMessage({ type: 'render', id, module, text, sampleRate: RENDER_RATE, limits: RENDER_LIMITS });
    if (signal) signal.addEventListener('abort', () => { const j = ws.jobs.get(id); if (j) { ws.jobs.delete(id); j.reject(Object.assign(new Error('the render was stopped'), { name: 'AbortError' })); } }, { once: true });
    return { result: await p, via: 'worker' };
  }
  const { renderProgram } = await import('./beatscript-render.js');
  return { result: await renderProgram({ module, text, sampleRate: RENDER_RATE, limits: RENDER_LIMITS, onProgress, signal }), via: 'page' };
}

// ---- the render's playback: an <audio> on a blob: WAV, or Web Audio on the same samples

/** An <audio> element playing `url`, or null when the page will not load it (a CSP without blob: in media-src). */
function audioPlayer(url) {
  return new Promise((resolve) => {
    const a = new Audio();
    a.preload = 'auto';
    let done = false;
    const end = (ok) => { if (done) return; done = true; clearTimeout(t); a.onloadedmetadata = a.onerror = null; resolve(ok ? a : null); };
    const t = setTimeout(() => end(false), BLOB_PROBE_MS);
    a.onloadedmetadata = () => end(Number.isFinite(a.duration) && a.duration > 0);
    a.onerror = () => end(false);
    a.src = url;
  });
}

/** The same interface for both: play(), pause(), seek(t), time, duration, loop, paused, onended. */
function elementTransport(a) {
  const tr = {
    kind: 'audio',
    el: a,
    get time() { return a.currentTime; },
    get duration() { return a.duration; },
    get paused() { return a.paused; },
    set loop(v) { a.loop = !!v; },
    get loop() { return a.loop; },
    onended: null,
    play: () => a.play(),
    pause: () => a.pause(),
    seek: (t) => { a.currentTime = Math.max(0, Math.min(t, a.duration || 0)); },
    dispose: () => { a.pause(); a.removeAttribute('src'); a.load(); },
  };
  a.addEventListener('ended', () => { if (tr.onended) tr.onended(); });
  return tr;
}

function bufferTransport(getCtx, getOut, buffer) {
  let src = null;
  let offset = 0;
  let startedAt = 0;
  let loop = false;
  const duration = buffer.duration;
  const stopSrc = () => { if (src) { src.onended = null; try { src.stop(); } catch { /* not started */ } src.disconnect(); src = null; } };
  const tr = {
    kind: 'webaudio',
    get time() {
      if (!src) return offset;
      const t = getCtx().currentTime - startedAt;
      return loop ? t % duration : Math.min(t, duration);
    },
    duration,
    get paused() { return !src; },
    set loop(v) {
      const t = tr.time;
      loop = !!v;
      if (src) { src.loop = loop; startedAt = getCtx().currentTime - t; }
    },
    get loop() { return loop; },
    onended: null,
    async play() {
      const ctx = getCtx();
      await ctx.resume();
      stopSrc();
      if (offset >= duration) offset = 0;
      src = ctx.createBufferSource();
      src.buffer = buffer;
      src.loop = loop;
      src.connect(getOut());
      src.onended = () => { src = null; offset = 0; if (tr.onended) tr.onended(); };
      startedAt = ctx.currentTime - offset;
      src.start(0, offset);
    },
    pause() { offset = tr.time; stopSrc(); },
    seek(t) {
      const was = !!src;
      offset = Math.max(0, Math.min(t, duration));
      if (was) { stopSrc(); tr.play(); }
    },
    dispose() { stopSrc(); offset = 0; },
  };
  return tr;
}

export class BeatscriptTrack extends HTMLElement {
  constructor() {
    super();
    this.ctx = null;
    this.out = null;
    this.engine = null;
    this.run = null;
    this.btw = null;
    this.sections = [];
    this.timer = 0;
    this.playing = false;
    this.busy = false;
    this.suspendTimer = 0;
    /** The decks whose nets the engine has installed. */
    this.installed = new Set();
    // render mode
    this.rendered = null; // {plan, sections, frames, sampleRate, ms, via, playback, cid}
    this.transport = null;
    this.blob = null;
    this.blobUrl = null;
    this.buffer = null;
    this.abort = null;
    this.progress = null;
  }

  /** 'live' (the default) or 'render'. */
  get mode() {
    return this.getAttribute('mode') === 'render' ? 'render' : 'live';
  }

  connectedCallback() {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    const s = styles();
    if (s) root.adoptedStyleSheets = [s];
    this.text = sourceOf(this);
    const site = (this.getAttribute('site') || SITE).replace(/\/+$/, '');
    this.btn = h('button', { type: 'button', class: 'play', 'aria-label': 'Play', part: 'play' }, '▶');
    this.sec = h('span', {}, 'not playing');
    this.bar = h('span', { class: 'dim' });
    this.state = h('span', { class: 'state', role: 'status', 'aria-live': 'polite' });
    this.linkLine = h('p', { class: 'links' });
    this.seek = h('input', { type: 'range', min: '0', max: '1', step: '0.01', value: '0', 'aria-label': 'Seek', part: 'seek', disabled: '' });
    this.time = h('span', { part: 'time' }, '0:00 / 0:00');
    this.loopBox = h('input', { type: 'checkbox', part: 'loop' });
    this.transportRow = h('p', { class: 'transport', hidden: '' }, this.seek, this.time, h('label', {}, this.loopBox, 'loop'));
    const name = nameOf(this.text) || 'BeatScript';
    this.code = h('code');
    this.copyBtn = h('button', { type: 'button', class: 'copy', part: 'copy', 'aria-label': 'Copy the program' }, 'Copy');
    this.source = h('div', { class: 'src', part: 'source', hidden: '' }, this.copyBtn, h('pre', {}, this.code));
    root.replaceChildren(h('div', { class: 'card', part: 'card' }, this.btn, h('div', { class: 'body' }, h('p', { class: 'name' }, name), h('p', { class: 'pos' }, this.sec, ' ', this.bar, ' ', this.state), this.transportRow, this.linkLine)), this.source);
    this.copyBtn.addEventListener('click', () => this.copy());
    if (this.hasAttribute('highlight')) this.showSource();
    this.site = site;
    this.btn.addEventListener('click', () => this.toggle());
    this.seek.addEventListener('input', () => {
      if (!this.transport) return;
      this.transport.seek(Number(this.seek.value));
      this.paint();
    });
    this.loopBox.addEventListener('change', () => { if (this.transport) this.transport.loop = this.loopBox.checked; });
    if (this.mode === 'render') this.transportRow.hidden = false;
    // the CID is known once the module is loaded (its check names it); until then, the expected one (a link only when
    // the site has it stored: the self-sealing link needs the module to pack the program)
    const want = this.getAttribute('cid');
    if (want && CID_RE.test(want)) this.showLinks(want, null);
  }

  disconnectedCallback() {
    this.stop();
    if (this.abort) this.abort.abort();
    clearTimeout(this.suspendTimer);
    // the render is kept (stop() paused it at 0): a Web Audio transport's AudioBuffer plays on any context, and it asks
    // for this.ctx (a new one, from the next click) when it plays again
    if (this.ctx) { try { this.ctx.close(); } catch { /* closed */ } }
    this.ctx = null;
    this.out = null;
  }

  say(t) {
    this.state.textContent = t;
  }

  /** The `highlight` attribute: the text, its comments coloured now, the rest once the engine's highlighter is loaded. */
  showSource() {
    this.source.hidden = false;
    paintCode(this.code, this.text, plainSpans(this.text));
    this.highlighted = 'comments';
    loadModule().then(({ btw }) => {
      paintCode(this.code, this.text, btw.highlight(this.text));
      this.highlighted = 'engine';
    }).catch(() => { /* the comments stay coloured; the card says why it cannot play when it is pressed */ });
  }

  /** Copy the program (from a click): the clipboard, else the text selected for the person to copy. */
  async copy() {
    const done = (t) => {
      this.copyBtn.textContent = t;
      clearTimeout(this.copyTimer);
      this.copyTimer = setTimeout(() => { this.copyBtn.textContent = 'Copy'; }, 2000);
    };
    try {
      if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('no clipboard');
      await navigator.clipboard.writeText(this.text);
      this.copied = 'clipboard';
      done('Copied');
      return;
    } catch { /* refused (a frame without clipboard-write, an old browser): select it instead */ }
    try {
      const sel = window.getSelection();
      const r = document.createRange();
      r.selectNodeContents(this.code);
      sel.removeAllRanges();
      sel.addRange(r);
      let ok = false;
      try { ok = document.execCommand && document.execCommand('copy'); } catch { ok = false; }
      this.copied = ok ? 'command' : 'selected';
      done(ok ? 'Copied' : 'Selected: press Ctrl+C');
    } catch {
      this.copied = 'failed';
      done('Copy refused');
    }
  }

  /**
   * The CID and "Open on beatscript.app". Both are the self-sealing share link (`url`: the program is in it, so it opens
   * on the site whether or not the site stored it); with `stored`, the CID links to /p/<CID>. With no link yet (before
   * the module packs one), the CID is plain text.
   */
  showLinks(cid, url) {
    const stored = this.hasAttribute('stored');
    const page = `${this.site}/p/${cid}`;
    const cidHref = stored ? page : url;
    const label = cid.slice(0, 10) + '…';
    const kids = [cidHref ? h('a', { href: cidHref, target: '_blank', rel: 'noopener', part: 'cid' }, label) : h('span', { part: 'cid', title: cid }, label)];
    const open = url || (stored ? page : null);
    if (open) kids.push(' · ', h('a', { href: open, target: '_blank', rel: 'noopener', part: 'open' }, 'Open on beatscript.app'));
    this.linkLine.replaceChildren(...kids);
  }

  paint() {
    let m;
    if (this.mode === 'render' && this.rendered && this.transport) {
      const t = this.transport.time;
      m = this.playing ? positionModel(statusAt(t, this.rendered.plan, this.rendered.sections), this.rendered.sections, true) : { sec: t > 0 ? 'paused' : 'not playing', bar: '' };
      if (document.activeElement !== this || this.shadowRoot.activeElement !== this.seek) this.seek.value = String(t);
      this.time.textContent = `${clock(t)} / ${clock(this.transport.duration)}`;
    } else {
      const st = this.run && this.playing ? this.run.status() : null;
      m = positionModel(st, this.sections, this.playing);
    }
    if (!this.progress) {
      this.sec.textContent = m.sec;
      this.bar.textContent = m.bar;
    }
    this.btn.textContent = this.playing ? (this.mode === 'render' ? '❚❚' : '■') : '▶';
    this.btn.classList.toggle('playing', this.playing);
    this.btn.setAttribute('aria-pressed', String(this.playing));
    this.btn.setAttribute('aria-label', this.playing ? (this.mode === 'render' ? 'Pause' : 'Stop') : 'Play');
  }

  toggle() {
    if (this.abort) { this.abort.abort(); return; }
    if (this.busy) return;
    if (this.playing) {
      if (this.mode === 'render') this.pause();
      else this.stop();
    } else this.start();
  }

  /** The AudioContext and the element's output (every sound goes through `out`), made from a click. */
  audio() {
    if (!this.ctx) {
      this.ctx = new AudioContext({ latencyHint: 'playback' });
      this.out = null;
    }
    if (!this.out) {
      this.out = this.ctx.createGain();
      this.out.connect(this.ctx.destination);
    }
    clearTimeout(this.suspendTimer);
    this.suspendTimer = 0;
    return this.ctx.resume();
  }

  /** The module, the check, the CID and the links: {module, btw, rep}. */
  async prepare() {
    const { module, btw } = await loadModule();
    this.btw = btw;
    const rep = btw.check(this.text);
    if (!rep || !rep.ok) {
      const d = rep && rep.diagnostics && rep.diagnostics.find((x) => x.severity === 'error');
      throw new Error(d ? `line ${d.line}: ${d.message}` : 'the program does not check');
    }
    const want = this.getAttribute('cid');
    if (want && want !== rep.cid) throw new Error(`refused: the text is ${rep.cid}, not ${want}`);
    const sh = btw.share(this.text);
    this.showLinks(rep.cid, sh && sh.ok ? sh.url.replace(/^https?:\/\/[^/#]+/, this.site) : null);
    this.sections = rep.sections || [];
    return { module, btw, rep };
  }

  async start() {
    if (this.mode === 'render') return this.startRender();
    this.busy = true;
    const resumed = this.audio();
    this.say('loading…');
    try {
      await resumed;
      const { module, rep } = await this.prepare();
      // a fresh engine for every Play: a stop disposed the last one (its decks and what it had scheduled), so this
      // one starts clean, from bar 1
      if (!this.engine) {
        this.installed = new Set();
        this.engine = await createEngine({ context: this.ctx, module, destination: this.out });
        this.engine.on('error', (e) => this.say(e.message));
        this.engine.on('grid', () => this.tick());
        this.engine.on('installed', (e) => this.installed.add(e.deck));
      }
      if (!this.run) this.run = this.btw.newRun();
      const ev = this.run.eval(this.text);
      if (!ev || !ev.ok) throw new Error('it does not play');
      this.playing = true;
      this.say('');
      this.timer = setInterval(() => this.tick(), TICK_MS);
      this.tick();
      this.dispatchEvent(new CustomEvent('play', { detail: { cid: rep.cid } }));
    } catch (e) {
      this.say(String(e.message || e));
    } finally {
      this.busy = false;
      this.paint();
    }
  }

  async startRender() {
    this.busy = true;
    const resumed = this.audio();
    try {
      await resumed;
      if (!this.rendered) await this.renderNow();
      if (!this.rendered) return;
      this.transport.loop = this.loopBox.checked;
      try {
        await this.transport.play();
      } catch (e) {
        // an <audio> that the browser will not start without a fresh gesture (the render outlasted the click's)
        if (e && e.name === 'NotAllowedError') { this.say('rendered: press play'); return; }
        throw e;
      }
      this.playing = true;
      this.say('');
      clearInterval(this.timer);
      this.timer = setInterval(() => this.paint(), 100);
      this.dispatchEvent(new CustomEvent('play', { detail: { cid: this.rendered.cid } }));
    } catch (e) {
      this.say(String(e.message || e));
    } finally {
      this.busy = false;
      this.paint();
    }
  }

  /** Render one pass, encode it, and get its playback ready (the first Play in render mode). */
  async renderNow() {
    this.say('loading…');
    const { module, rep } = await this.prepare();
    const plan = renderPlan(rep, RENDER_LIMITS);
    this.abort = new AbortController();
    this.progress = { frames: 0, total: 1 };
    this.sec.textContent = 'rendering';
    this.bar.textContent = '';
    this.say('0%');
    const onProgress = (p) => {
      this.progress = p;
      this.sec.textContent = 'rendering';
      this.bar.textContent = `bar ${p.bars}/${p.totalBars}`;
      this.say(`${Math.floor(100 * p.fraction)}%`);
    };
    let r;
    let via;
    try {
      ({ result: r, via } = await renderOnce(module, this.text, onProgress, this.abort.signal));
    } catch (e) {
      if (e && e.name === 'AbortError') { this.say('render stopped'); return; }
      throw e;
    } finally {
      this.abort = null;
      this.progress = null;
    }
    this.say('encoding…');
    const bytes = wav(r.left, r.right, r.sampleRate);
    this.blob = new Blob([bytes], { type: 'audio/wav' });
    this.blobUrl = URL.createObjectURL(this.blob);
    let playback = 'audio';
    const a = await audioPlayer(this.blobUrl);
    if (a) {
      this.transport = elementTransport(a);
    } else {
      // no blob: media here: the same samples through Web Audio
      playback = 'webaudio';
      const buf = this.ctx.createBuffer(2, r.frames, r.sampleRate);
      buf.copyToChannel(r.left, 0);
      buf.copyToChannel(r.right, 1);
      this.buffer = buf;
      this.transport = bufferTransport(() => this.ctx, () => this.out, buf);
    }
    this.transport.onended = () => this.ended();
    this.rendered = { plan, sections: rep.sections || [], frames: r.frames, sampleRate: r.sampleRate, seconds: r.seconds, bars: r.bars, loops: r.loops, ms: r.ms, via, playback, cid: rep.cid, notes: r.notes, lead: r.lead };
    this.loopBox.checked = !!r.loops;
    this.seek.max = String(r.seconds);
    this.seek.removeAttribute('disabled');
    this.say('');
    this.dispatchEvent(new CustomEvent('rendered', { detail: { cid: rep.cid, seconds: r.seconds, frames: r.frames, bars: r.bars, ms: r.ms, via, playback } }));
  }

  /** Render mode: hold the place. */
  pause() {
    if (!this.transport || !this.playing) return;
    this.transport.pause();
    this.playing = false;
    clearInterval(this.timer);
    this.timer = 0;
    this.dispatchEvent(new CustomEvent('pause'));
    this.paint();
  }

  /** The render played to its end (not looping). */
  ended() {
    if (!this.playing) return;
    this.playing = false;
    clearInterval(this.timer);
    this.timer = 0;
    if (this.transport) this.transport.seek(0);
    this.dispatchEvent(new CustomEvent('stop'));
    this.paint();
  }

  tick() {
    if (!this.playing || !this.run || !this.engine) return;
    this.engine.apply(this.run.service(this.engine.grid()));
    const st = this.run.status();
    if (st && st.ended) return this.stop();
    this.paint();
  }

  /** Silent at once. Live: the engine (its decks and what it had scheduled) is let go, so the next Play starts clean from
   * bar 1, and the context sleeps soon after. Render: the playback pauses and goes back to the start. */
  stop() {
    clearInterval(this.timer);
    this.timer = 0;
    const was = this.playing;
    this.playing = false;
    if (this.engine) {
      try { if (this.run && was) this.engine.apply(this.run.stop()); } catch { /* going */ }
      try { this.engine.dispose(); } catch { /* gone */ }
      this.engine = null;
    }
    if (this.run) { try { this.run.free(); } catch { /* freed */ } this.run = null; }
    if (this.transport) { this.transport.pause(); this.transport.seek(0); }
    if (this.ctx && this.ctx.state === 'running') {
      clearTimeout(this.suspendTimer);
      const ctx = this.ctx;
      this.suspendTimer = setTimeout(() => { if (!this.playing && ctx.state === 'running') ctx.suspend().catch(() => {}); }, SUSPEND_AFTER_MS);
    }
    if (was) this.dispatchEvent(new CustomEvent('stop'));
    if (this.btn) this.paint();
  }

  /** The render's WAV (a Blob) once render mode has rendered, else null. */
  get wav() {
    return this.blob;
  }

  /** What the tests read: playing, the section and bar line, the engine's composer, the state line, the mode, the render. */
  get report() {
    return {
      playing: this.playing,
      mode: this.mode,
      sec: this.sec && this.sec.textContent,
      bar: this.bar && this.bar.textContent,
      state: this.state && this.state.textContent,
      composer: this.engine ? this.engine.composer : null,
      installed: [...this.installed].sort(),
      cid: this.run ? this.run.cid() : this.rendered ? this.rendered.cid : null,
      time: this.transport ? this.transport.time : null,
      render: this.rendered ? { ...this.rendered, plan: undefined, sections: undefined } : this.progress ? { progress: { ...this.progress } } : null,
      highlighted: this.highlighted || null,
      copied: this.copied || null,
      links: this.linkLine ? [...this.linkLine.querySelectorAll('a')].map((a) => ({ part: a.getAttribute('part'), href: a.getAttribute('href') })) : [],
    };
  }
}

if (typeof customElements !== 'undefined' && !customElements.get('beatscript-track')) customElements.define('beatscript-track', BeatscriptTrack);
