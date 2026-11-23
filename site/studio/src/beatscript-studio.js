// <beatscript-studio>: the BeatScript editor, its checker and the player in one element, self-contained.
//
//   <script type="module" src="https://cdn.jsdelivr.net/npm/@stackdump/beatscript-studio@0.1.0/beatscript-studio.js"></script>
//   <beatscript-studio>
//     <script type="text/beatscript">
//   tempo 120
//   deck A: kick x ~ x ~
//     </script>
//   </beatscript-studio>
//
// The config is the one every surface reads (embed-config.js, site/web/embed-config.schema.json): the `config` attribute
// or a child <script type="application/json" class="beatscript-config">, with single attributes over it (audio,
// features, capabilities, theme, compact, title, site, bridge, ...). The host is asked what it can do (detectHost: a
// Claude artifact's claude.use(), WebAssembly, clipboard), and what it cannot is dropped with a note under the editor.
//
// It runs the same engine as <beatscript-track>, from the same package files: the module is compiled once from the
// inlined base64 (wasm/btw_bg.wasm.b64.js), nothing is fetched from beatscript.app, and sound starts only from a click.
// The deterministic tools (check, fmt, share, open, generate, ...) run in the page; the ones that need the host (compose,
// store, library, jam) appear only when their capability resolved. element.tools is the MCP-shaped tool surface over
// all of them (tool-surface.js); with `bridge`, a parent frame can call it over postMessage. With dictation, a mic beside
// the compose prompt types what the viewer says; with tts, a speaker on the status line reads the problems aloud.
//
// The styles are a constructed stylesheet in the shadow root (studio-css.js), so a page whose style-src is 'self' shows
// them. The editor is the site's own (js/editor.js, unchanged): it builds its layers inside the shadow root and measures
// them there.

import { makeFacade } from './js/wasm.js';
import { createEngine } from './js/engine.js';
import { positionModel } from './js/embed-model.js';
import { Editor } from './js/editor.js';
import { debounce, h, MAX_LINK_CHARS } from './js/util.js';
import { SEEDED_GENRES } from './js/station-model.js';
import { wav } from './render-model.js';
import { fromElement, normalize, detectHost, resolve, DEFAULT_SITE } from './embed-config.js';
import { createToolSurface } from './tool-surface.js';
import { studioSheet } from './studio-css.js';

const TICK_MS = 25;
/** How long typing pauses before the text is checked again. */
const CHECK_MS = 250;
const RENDER_RATE = 44100;
/** After a stop, the context is suspended this much later (the output is already cut). */
const SUSPEND_AFTER_MS = 300;
/** How long a blob: WAV may take to load before Web Audio plays it instead (an artifact's CSP refuses blob: media). */
const BLOB_PROBE_MS = 4000;
/** The most tracks the station tool gives at once. */
const STATION_MAX = 16;
const CID_ONLY = 'this item is only a CID: it needs its text (the site\'s stored program) to play here';
const NO_STATION = 'a station needs a module with btw_station (not in this build); open /radio on the site';
/** The most tts reads in one speak() (tts.d.ts: a text of more than 2,000 characters is refused). */
const SPEAK_MAX = 2000;
/** The status line for a dictation or tts call that failed, by its error code; any other code is "... failed". */
const VOICE_SAYS = {
  busy: 'not read aloud while the microphone is in use',
  rate_limited: 'too many at once: try again shortly',
  no_gesture: 'press again in a moment',
};

/** What the editor holds when the config and the markup give no program. */
export const STARTER = `#! name: a first beat
tempo 120
key am
let four = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
let backs = ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~
let offs = ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
section verse 4 bars {
  deck A: kick four kit=drums-808 | snare backs | hihat offs
  deck B: melody a4 ~ c5 ~ e5 ~ c5 ~ a4 ~ g4 ~ a4 ~ ~ ~
}
arrange verse
`;

/** The module, once per page: compiled from the inlined bytes, as beatscript-track.js's loadModule does. */
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
        throw new Error(/unsafe-eval|Content Security/i.test(String(e)) ? 'this page does not allow WebAssembly, so the program cannot be checked or played here' : String(e.message || e));
      }
      glue.initSync({ module });
      return { module, btw: makeFacade(glue) };
    })();
    shared.catch(() => { shared = null; });
  }
  return shared;
}

/** `#! name:`, else ''. */
const nameOf = (text) => { const m = /^#!\s*name:\s*(.+)$/m.exec(text || ''); return m ? m[1].trim() : ''; };
/** A file name from the program's name. */
const fileBase = (text) => (nameOf(text).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'beatscript').slice(0, 60);
/** The text of an MCP CallToolResult. */
const resultText = (r) => ((r && r.content) || []).filter((c) => c && c.type === 'text').map((c) => c.text).join('\n');
/** A set item's program text, or null (an item with only a CID). */
const itemText = (it) => (it && typeof it.text === 'string' && it.text.trim() ? it.text : null);
/** A set item's label: its title, else its `#! name:`, else "item N". */
const itemTitle = (it, i) => (it && it.title) || nameOf(itemText(it)) || `item ${i + 1}`;

/** An <audio> that plays `url`, or null when the page will not load it. */
function audioProbe(a, url) {
  return new Promise((resolve) => {
    let done = false;
    const end = (ok) => { if (done) return; done = true; clearTimeout(t); a.onloadedmetadata = a.onerror = null; resolve(ok); };
    const t = setTimeout(() => end(false), BLOB_PROBE_MS);
    a.onloadedmetadata = () => end(Number.isFinite(a.duration) && a.duration > 0);
    a.onerror = () => end(false);
    a.src = url;
  });
}

export class BeatscriptStudio extends HTMLElement {
  constructor() {
    super();
    this.tools = null;
    this.btw = null;
    this.module = null;
    this.rep = null;
    this.notes = [];
    this.caps = {};
    this.ctx = null;
    this.out = null;
    this.engine = null;
    this.run = null;
    this.timer = 0;
    this.playing = false;
    this.busy = false;
    this.installed = new Set();
    this.sections = [];
    this.panelFor = null;
    this.unserve = null;
    this.set = null;
    this.setAt = 0;
    this.setEl = null;
    this.askEl = null;
    this.guessEl = null;
    this.unlisten = [];
    this.voiceOff = new Set();
  }

  connectedCallback() {
    // put back on the page: the tools (closed when it was taken off) are made again, and dictation and tts heard again
    if (this.shadowRoot) { if (this.btw) this.makeTools(); this.listen(); return; }
    const root = this.attachShadow({ mode: 'open' });
    const s = studioSheet();
    if (s) root.adoptedStyleSheets = [s];
    const n = this.readConfig();
    this.config = n.config;
    this.notes = n.notes;
    this.site = String(this.config.site || DEFAULT_SITE).replace(/\/+$/, '');
    const src = this.config.source || {};
    const set = Array.isArray(src.set) && src.set.length ? src.set : null;
    this.text = (set ? itemText(set[0]) : itemText(src)) || STARTER;
    if (!src.text && src.cid) this.notes.push({ what: 'source', status: 'degraded', message: 'a CID alone needs its text (the site\'s stored program) to open here, so the studio shows a starter program' });

    this.btns = {};
    const tb = (...a) => this.button(...a);
    this.nameEl = h('span', { class: 'name' });
    this.bar = h('div', { class: 'bar', role: 'toolbar', 'aria-label': 'BeatScript' },
      tb('play', '▶ Play', 'Play (Ctrl+Enter)', () => this.toggle()),
      tb('render', 'Render', 'Render the whole track, then play it', () => this.renderAndPlay()),
      tb('fmt', 'fmt', 'Format the program', () => this.fmt()),
      tb('share', 'share', 'Copy the self-sealing link', () => this.share()),
      tb('open', 'open', 'Open a link or CID', () => this.togglePanel('open')),
      tb('generate', 'generate', 'Generate a program from a genre and a seed', () => this.togglePanel('generate')),
      tb('exportBtw', '.btw', 'Export the program as text', () => this.exportBtw()),
      tb('exportWav', 'WAV', 'Render and export a WAV', () => this.exportWav()),
      tb('compose', 'compose', 'Ask Claude for a program or a change', () => this.togglePanel('compose')),
      tb('store', 'store', 'Store the program through your BeatScript connector', () => this.store()),
      tb('library', 'library', 'Your saved programs', () => this.togglePanel('library')),
      tb('jam', 'jam', 'Send the program to everyone in the room', () => this.jam()),
      this.nameEl,
      tb('help', '?', 'Show the cheatsheet', () => this.togglePanel('help')));
    this.panel = h('div', { class: 'panel', part: 'panel', hidden: true });
    this.edRoot = h('div', { class: 'ed', part: 'editor' });
    this.problems = h('ul', { class: 'problems', part: 'problems', 'aria-label': 'Problems', hidden: true });
    this.sec = h('span', {}, 'not playing');
    this.barLabel = h('span', {});
    this.checkLabel = h('span', {});
    this.state = h('span', { role: 'status', 'aria-live': 'polite' });
    this.dl = h('a', { class: 'dl', part: 'download', hidden: true });
    this.status = h('div', { class: 'status', part: 'status' }, this.sec, this.barLabel, h('span', { class: 'spacer' }), this.dl, this.state, this.checkLabel);
    this.audioEl = h('audio', { controls: true, part: 'audio', hidden: true });
    this.notesEl = h('p', { class: 'notes', part: 'notes', 'aria-label': 'What is not available here' });
    this.wrap = h('div', { class: 'studio', 'data-theme': this.config.theme || 'dark' }, this.bar, this.panel, this.edRoot, this.problems, this.status, this.audioEl, this.notesEl);
    if (this.config.compact) this.wrap.classList.add('compact');
    if (this.config.height) this.wrap.style.setProperty('--ed-h', `${Math.max(80, this.config.height - 140)}px`);
    if (this.config.width) this.style.maxWidth = `${this.config.width}px`;
    root.replaceChildren(this.wrap);

    // the editor measures its layers when it is made, so it is made once they are in the (connected) shadow root
    this.editor = new Editor(this.edRoot, {
      onInput: (text) => { this.text = text; this.checkSoon(); this.dispatchEvent(new CustomEvent('change', { detail: { text } })); },
      onEval: () => this.evaluate(),
      onStop: () => this.stop(),
    });
    this.editor.setText(this.text);
    this.checkSoon = debounce(() => this.check(), CHECK_MS);
    if (set) this.setList(set);
    this.paintBar();
    this.paint();
    this.boot();
  }

  /** A button kept in this.btns, which paintBar shows or hides by name. */
  button(name, label, title, fn) {
    return (this.btns[name] = h('button', { type: 'button', class: name === 'play' ? 'tb play' : 'tb', part: name, title, 'aria-label': title, onclick: fn }, label));
  }

  disconnectedCallback() {
    this.stop();
    clearTimeout(this.suspendTimer);
    this.dropTools();
    this.unhear();
    if (this.ctx) { try { this.ctx.close(); } catch { /* closed */ } }
    this.ctx = null;
    this.out = null;
  }

  // ---- a set: the source's items, in order ---------------------------------------------------------------------

  /** The set list in the toolbar (a <select> of the items' titles); the first item is in the editor. */
  setList(items) {
    this.set = items;
    this.setAt = 0;
    const opts = items.map((it, i) => h('option', { value: String(i) }, itemTitle(it, i)));
    const sel = h('select', { class: 'set', part: 'set', 'aria-label': 'Set list', onchange: () => this.pick(Number(sel.value)) }, ...opts);
    if (this.setEl) this.setEl.replaceWith(sel);
    else this.bar.insertBefore(sel, this.nameEl);
    this.setEl = sel;
    if (!itemText(items[0])) this.say(CID_ONLY);
  }

  /** Item i into the editor; false (and why, on the status line) when it has only a CID. */
  pick(i) {
    const it = this.set && this.set[i];
    if (!it) return false;
    this.setAt = i;
    this.setEl.value = String(i);
    const t = itemText(it);
    if (!t) { this.say(CID_ONLY); return false; }
    if (this.playing) this.stop();
    this.load(t);
    this.say('');
    return true;
  }

  /** The program played to its end: the set's next item plays (the context is running since the first click). */
  ended() {
    this.stop();
    if (this.set && this.setAt + 1 < this.set.length && this.pick(this.setAt + 1)) this.start();
  }

  /** A station source: its tracks as a set, through the station tool where this module has btw_station. */
  async tune(src) {
    const t = this.tools && this.tools.list().find((x) => x.name === 'station');
    if (!t || !t.available) { this.notes.push({ what: 'source', status: 'unsupported', message: NO_STATION }); return; }
    const r = await this.tools.call('station', { station: src.station, n: src.n || 0, count: Math.min(STATION_MAX, src.count || 1) });
    const tracks = (r && !r.isError && r.structuredContent && r.structuredContent.tracks) || [];
    const items = tracks.filter((x) => x && (typeof x.text === 'string' || x.cid)).map((x) => ({ text: x.text, cid: x.cid, title: x.title || x.name }));
    if (!items.length) { this.notes.push({ what: 'source', status: 'unsupported', message: `the station gave no tracks (${resultText(r) || 'no answer'})` }); return; }
    this.setList(items);
    if (itemText(items[0])) this.load(items[0].text);
  }

  /** {config, notes}: the element's config, normalized as a studio (an invalid one says why and uses the defaults). */
  readConfig() {
    let raw;
    try { raw = fromElement(this); } catch { raw = {}; }
    // fromElement gives JSON that does not parse, or is not an object, as it is, for normalize to say what is wrong
    const obj = raw && typeof raw === 'object' && !Array.isArray(raw);
    const n = normalize(obj ? { ...raw, kind: 'studio' } : raw);
    if (n.ok) return { config: n.config, notes: (n.warnings || []).map((w) => ({ what: 'config', status: 'degraded', message: w })) };
    const text = obj && raw.source && typeof raw.source.text === 'string' ? raw.source.text : null;
    const d = normalize({ kind: 'studio', ...(text ? { source: { text } } : {}) });
    const why = (n.errors || []).map((e) => `${e.path || 'config'} ${e.message}`).join('; ');
    return { config: d.config, notes: [{ what: 'config', status: 'degraded', message: `the config is not valid (${why}), so the studio uses its defaults` }] };
  }

  /** The module, the host and the tools; each lights up what it can when it arrives. */
  async boot() {
    const asked = this.config.capabilities || [];
    // the db's library is the viewer's own: the user namespace says who that is
    const host = detectHost(window, { want: asked.includes('db') ? [...asked, 'user'] : asked }).catch(() => null);
    let failed = null;
    try {
      const { module, btw } = await loadModule();
      this.module = module;
      this.btw = btw;
      if ((this.config.features || []).includes('highlight')) this.editor.setHighlighter((t) => btw.highlight(t));
      this.check();
    } catch (e) {
      failed = String(e.message || e);
      this.say('cannot check or play here');
    }
    const found = await host;
    if (found) {
      const r = resolve(this.config, found);
      this.config = r.config;
      this.notes.push(...(r.notes || []));
      const kept = new Set(this.config.capabilities || []);
      for (const [k, v] of Object.entries(found.caps || {})) if (v && (kept.has(k) || (k === 'user' && kept.has('db')))) this.caps[k] = v;
    }
    // resolve says it when the CSP refuses WebAssembly; any other reason the module did not load is said here
    if (failed && !(found && found.wasm === false)) this.notes.push({ what: 'wasm', status: 'unsupported', message: failed });
    // taken off the page while loading: the tools are made when it is put back (connectedCallback)
    if (this.btw && this.isConnected) this.makeTools();
    this.voice();
    const src = this.config.source || {};
    if (src.station !== undefined) await this.tune(src);
    this.paintBar();
    this.paintNotes();
    this.dispatchEvent(new CustomEvent('ready', { detail: this.report }));
  }

  /** Close the tools: the bridge stops answering and the room's listener stops. */
  dropTools() {
    if (this.unserve) { try { this.unserve(); } catch { /* gone */ } this.unserve = null; }
    if (this.tools) { try { this.tools.close(); } catch { /* closed */ } }
  }

  /** element.tools, over this studio's engine and the capabilities that resolved; with `bridge`, served to the parent. */
  makeTools() {
    // one surface at a time, so the bridge never answers twice
    this.dropTools();
    this.tools = createToolSurface({
      btw: this.btw,
      caps: this.caps,
      site: this.site,
      render: (text) => this.renderText(text),
      encodeWav: wav,
      onLoad: (text) => this.load(text),
      onJam: (msg) => this.jammed(msg),
    });
    if (this.config.bridge) this.unserve = this.tools.serve(window);
  }

  // ---- dictation and tts -------------------------------------------------------------------------------------------

  /** The mic (beside the compose prompt) and the speaker (on the status line), made only when their capability resolved. */
  voice() {
    if (this.caps.dictation && !this.btns.dictate) {
      this.button('dictate', 'dictate', 'Dictate the prompt', () => this.dictate());
      this.guessEl = h('span', { class: 'guess', part: 'guess' });
    }
    if (this.caps.tts && !this.btns.speak) this.status.insertBefore(this.button('speak', 'read aloud', 'Read the problems aloud', () => this.speakProblems()), this.checkLabel);
    // taken off the page while loading: it subscribes when it is put back (connectedCallback)
    if (this.isConnected) this.listen();
  }

  /** Subscribe to dictation and tts once, keeping each Unsubscribe for disconnectedCallback. */
  listen() {
    if (this.unlisten.length) return;
    const d = this.caps.dictation;
    const t = this.caps.tts;
    // onState calls back at once only when the state is not idle, so the buttons start idle
    this.paintVoice('dictate', 'idle');
    this.paintVoice('speak', 'idle');
    if (d) {
      this.unlisten.push(
        d.onPartial(({ text }) => { if (this.guessEl) this.guessEl.textContent = text; }),
        d.onFinal(({ text }) => this.dictated(text)),
        d.onState(({ state, reason }) => this.voiceState('dictate', state, reason)),
      );
    }
    if (t) this.unlisten.push(t.onState(({ state, reason }) => this.voiceState('speak', state, reason)));
  }

  /** Taken off the page: nothing more is heard, and a dictation or a reading under way stops. */
  unhear() {
    for (const u of this.unlisten.splice(0)) { try { u(); } catch { /* gone */ } }
    const { dictation: d, tts: t } = this.caps;
    if (d && d.active()) d.stop().catch(() => {});
    if (t && t.active()) t.stop().catch(() => {});
  }

  /** The mic: start() from the click itself, with nothing awaited first; a second click stops, once the last words are in. */
  async dictate() {
    const d = this.caps.dictation;
    if (!d) return;
    if (d.active()) { await d.stop().catch(() => {}); return; }
    d.start().catch((e) => this.voiceFailed('dictate', e));
  }

  /** A settled phrase, appended to the prompt as it comes (it carries its own leading space, trimmed on an empty field). */
  dictated(text) {
    const a = this.askEl;
    if (!a || !text) return;
    a.value = a.value ? a.value + text : text.trimStart();
  }

  /** The speaker: read the problems aloud (the list stays on screen); a second click stops the reading. */
  speakProblems() {
    const t = this.caps.tts;
    if (!t) return;
    if (t.active()) { t.stop().catch(() => {}); return; }
    this.say('');
    t.speak(this.problemsSpoken()).catch((e) => this.voiceFailed('speak', e));
  }

  /** "no problems", or "N problems. Line L: message. ...", as many lines as fit in SPEAK_MAX characters. */
  problemsSpoken() {
    const ps = this.problemList();
    if (!ps.length) return 'no problems';
    let s = `${ps.length} problem${ps.length === 1 ? '' : 's'}.`;
    for (const p of ps) {
      const line = ` Line ${p.line}: ${String(p.message).replace(/[\s.]+$/, '')}.`;
      if (s.length + line.length > SPEAK_MAX) break;
      s += line;
    }
    return s;
  }

  /** The mic or the speaker drawn from its capability's state: pressed, and a stop, while not idle. */
  paintVoice(name, state) {
    const b = this.btns[name];
    if (!b) return;
    const on = state !== 'idle';
    const [idle, busy, stop] = name === 'dictate' ? ['dictate', '■ listening', 'Stop dictating'] : ['read aloud', '■ reading', 'Stop reading'];
    b.textContent = on ? busy : idle;
    b.setAttribute('aria-pressed', String(on));
    b.setAttribute('aria-label', on ? stop : b.title);
    if (name === 'dictate' && !on && this.guessEl) this.guessEl.textContent = '';
  }

  /** onState: the button follows it; the viewer turning it off (or it not being offered) hides the control for good. */
  voiceState(name, state, reason) {
    this.paintVoice(name, state);
    if (state === 'idle' && (reason === 'not_granted' || reason === 'unavailable')) this.hideVoice(name);
  }

  /** A refusal hides the control for this page load, so it is never asked again; dismissed may ask on a later click. */
  voiceFailed(name, e) {
    const code = (e && e.code) || 'upstream_error';
    if (code === 'not_granted' || code === 'unavailable') return this.hideVoice(name);
    if (code === 'stopped' || code === 'dismissed') return;
    this.say(VOICE_SAYS[code] || `${name === 'dictate' ? 'dictation' : 'reading aloud'} failed`);
  }

  hideVoice(name) {
    this.voiceOff.add(name);
    this.paintBar();
  }

  say(t) {
    this.state.textContent = t;
  }

  // ---- the toolbar and the panel ---------------------------------------------------------------------------------

  /** Which buttons show: by the effective config's features, and the capability buttons only when theirs resolved. */
  paintBar() {
    const f = new Set(this.config.features || []);
    const compose = f.has('compose') && !!this.caps.sample;
    const show = {
      play: true,
      render: this.config.audio === 'render',
      fmt: f.has('editor'),
      share: f.has('share'),
      open: f.has('editor'),
      generate: f.has('editor'),
      exportBtw: f.has('download'),
      exportWav: f.has('download'),
      compose,
      store: !!this.caps.mcp,
      library: !!this.caps.db,
      jam: !!this.caps.room,
      help: true,
      // the mic is in the compose panel; the speaker reads the problems, which need the engine
      dictate: compose && !!this.caps.dictation && !this.voiceOff.has('dictate'),
      speak: !!this.caps.tts && !!this.btw && !this.voiceOff.has('speak'),
    };
    for (const [k, b] of Object.entries(this.btns)) b.hidden = !show[k];
    this.editor.ta.readOnly = !f.has('editor');
    this.nameEl.textContent = this.config.title || nameOf(this.text) || 'BeatScript';
  }

  /** The notes line: one sentence for each thing this host does not do as asked. */
  paintNotes() {
    const sentence = (m) => { const t = String(m || '').trim(); return t ? `${t[0].toUpperCase()}${t.slice(1)}${/[.!?]$/.test(t) ? '' : '.'}` : ''; };
    this.notesEl.textContent = this.notes.filter((n) => n.status !== 'works').map((n) => sentence(n.message)).filter(Boolean).join(' ');
  }

  /** Open the panel for `what` (its button pressed), or close it when it is already open. */
  togglePanel(what) {
    if (this.panelFor === what) return this.closePanel();
    this.closePanel();
    const kids = this.panelKids(what);
    if (!kids) return;
    this.panelFor = what;
    this.panel.replaceChildren(...kids);
    this.panel.hidden = false;
    if (this.btns[what]) this.btns[what].setAttribute('aria-expanded', 'true');
    const first = this.panel.querySelector('input, textarea, select');
    if (first) first.focus();
  }

  closePanel() {
    if (this.panelFor && this.btns[this.panelFor]) this.btns[this.panelFor].removeAttribute('aria-expanded');
    this.panelFor = null;
    this.panel.hidden = true;
    this.panel.replaceChildren();
  }

  /** A line in the open panel. */
  panelSay(text, err = false) {
    let m = this.panel.querySelector('.msg');
    if (!m) { m = h('div', { class: 'msg' }); this.panel.append(m); }
    m.className = err ? 'msg err' : 'msg';
    m.textContent = text;
  }

  panelKids(what) {
    if (what === 'open') {
      const inp = h('input', { type: 'text', class: 'grow', placeholder: 'a share link or a CID', 'aria-label': 'Link or CID' });
      const go = () => this.open(inp.value);
      inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
      return [inp, h('button', { type: 'button', class: 'tb', onclick: go }, 'Open')];
    }
    if (what === 'generate') {
      const genre = h('select', { 'aria-label': 'Genre' }, ...SEEDED_GENRES.map((g) => h('option', { value: g }, g)));
      const seed = h('input', { type: 'text', value: String(1 + Math.floor(Math.random() * 9999)), size: '8', 'aria-label': 'Seed' });
      return [genre, seed, h('button', { type: 'button', class: 'tb', onclick: () => this.generate(genre.value, seed.value) }, 'Generate')];
    }
    if (what === 'compose') {
      // one prompt field for the element's life, so dictation's last words land in it even after the panel closes
      const ask = this.askEl || (this.askEl = h('textarea', { class: 'grow', placeholder: 'what to write, or what to change in this program', 'aria-label': 'Ask' }));
      const go = h('button', { type: 'button', class: 'tb', onclick: () => this.compose(ask.value, go) }, 'Compose');
      return [ask, this.btns.dictate, go, this.guessEl].filter(Boolean);
    }
    if (what === 'library') {
      this.libraryList();
      return [h('button', { type: 'button', class: 'tb', onclick: () => this.librarySave() }, 'Save this program'), h('ul', {})];
    }
    if (what === 'help') {
      let t = '';
      try { t = this.btw ? String(this.btw.cheatsheet() || '') : ''; } catch { t = ''; }
      return [h('pre', { part: 'cheatsheet' }, t || 'The cheatsheet comes with the engine, which is not loaded here.')];
    }
    return null;
  }

  // ---- the text ----------------------------------------------------------------------------------------------------

  /** Put a program in the editor (open, generate, compose, the library, a jam). */
  load(text) {
    this.text = String(text || '');
    this.editor.replaceUndoable(this.text);
    this.paintBar();
    this.check();
  }

  /** Check the text: the problems list, the squiggles, the CID. */
  check() {
    if (!this.btw) return null;
    let rep;
    try { rep = this.btw.check(this.text); } catch (e) { rep = { ok: false, diagnostics: [{ line: 1, col: 1, severity: 'error', message: String(e.message || e) }] }; }
    this.rep = rep;
    this.sections = rep.sections || this.sections;
    const diags = (rep.diagnostics || []).filter((d) => d.severity === 'error' || d.severity === 'warning');
    this.editor.setDiagnostics(diags);
    this.problems.replaceChildren(...diags.map((d) => h('li', {}, h('button', { type: 'button', onclick: () => this.editor.jumpTo(d.line, d.col, d.end_col) },
      h('span', { class: 'pos' }, `${d.line}:${d.col}`), h('span', { class: `sev-${d.severity}` }, d.severity), ` ${d.message}`))));
    this.problems.hidden = diags.length === 0;
    const errors = diags.filter((d) => d.severity === 'error').length;
    this.checkLabel.replaceChildren(rep.ok ? h('span', { class: 'ok', title: rep.cid || '' }, `✓ ${String(rep.cid || '').slice(0, 10)}…`) : h('span', { class: 'bad' }, `${errors || 1} problem${errors === 1 ? '' : 's'}`));
    this.nameEl.textContent = this.config.title || nameOf(this.text) || 'BeatScript';
    this.dispatchEvent(new CustomEvent('check', { detail: { ok: !!rep.ok, cid: rep.cid || null, problems: this.problemList() } }));
    return rep;
  }

  problemList() {
    return ((this.rep && this.rep.diagnostics) || []).filter((d) => d.severity === 'error' || d.severity === 'warning').map((d) => ({ line: d.line, col: d.col, severity: d.severity, message: d.message }));
  }

  fmt() {
    if (!this.btw) return;
    const r = this.btw.fmt(this.text);
    if (r && r.ok && typeof r.text === 'string' && r.text !== this.text) this.load(r.text);
    else if (r && !r.ok) this.say('fix the problems first');
  }

  /** The self-sealing link (the program is in it): to the clipboard, else in a field selected for the reader to copy. */
  async share() {
    if (!this.btw) return;
    const sh = this.btw.share(this.text);
    if (!sh || !sh.ok) { this.say('fix the problems first'); return; }
    const url = sh.url.replace(/^https?:\/\/[^/#]+/, this.site);
    this.closePanel();
    const field = h('input', { type: 'text', class: 'grow', readonly: true, value: url, 'aria-label': 'Share link', part: 'share-link' });
    this.panelFor = 'share';
    this.panel.replaceChildren(field, h('a', { href: url, target: '_blank', rel: 'noopener' }, 'open'));
    this.panel.hidden = false;
    this.shared = { url, copied: false };
    try {
      if (!navigator.clipboard || !navigator.clipboard.writeText) throw new Error('no clipboard');
      await navigator.clipboard.writeText(url);
      this.shared.copied = true;
      this.panelSay('Copied the link.');
    } catch {
      field.focus();
      field.select();
      this.panelSay('Selected: press Ctrl+C to copy it.');
    }
  }

  /** A share link opens here (its program is in it, checked against its CID); a bare CID needs the site. */
  open(s) {
    const v = String(s || '').trim();
    if (!v || !this.btw) return;
    if (v.length > MAX_LINK_CHARS) { this.panelSay('That link is too long to open.', true); return; }
    let r;
    try { r = this.btw.open(v); } catch (e) { r = { ok: false, error: String(e.message || e) }; }
    if (!r || !r.ok || r.verified === false) {
      this.panelSay(r && r.cid && !/\./.test(v.split('#').pop()) ? 'A CID alone needs the site: paste the share link, which carries the program.' : `Not opened: ${String((r && r.error) || 'the link does not match its own CID').replace(/^refused:\s*/i, '')}.`, true);
      return;
    }
    this.load(r.canonical);
    this.closePanel();
  }

  generate(genre, seed) {
    if (!this.btw) return;
    if (!this.btw.generate) { this.panelSay('This engine has no generator.', true); return; }
    let r;
    try { r = this.btw.generate({ genre, seed: String(seed || '1').trim() || '1' }); } catch (e) { r = { ok: false, error: String(e.message || e) }; }
    if (!r || !r.text) { this.panelSay(`Not generated: ${(r && r.error) || 'the generator refused'}.`, true); return; }
    this.load(r.text);
    this.panelSay(`${genre}, seed ${r.seed != null ? r.seed : seed}.`);
  }

  // ---- the host's tools ------------------------------------------------------------------------------------------

  /** A tool's CallToolResult, or null before the tools are made. */
  tool(name, args) {
    return this.tools ? this.tools.call(name, args || {}) : Promise.resolve(null);
  }

  async compose(prompt, btn) {
    if (!String(prompt || '').trim()) return;
    btn.disabled = true;
    this.panelSay('Asking Claude…');
    try {
      // a program that checks reaches the editor through onLoad
      const r = await this.tool('compose', { prompt, text: this.text });
      const out = (r && r.structuredContent) || {};
      if (r && !r.isError && out.ok) this.panelSay(`Composed in ${out.rounds} round${out.rounds === 1 ? '' : 's'}.`);
      else this.panelSay(r && r.isError ? resultText(r) : 'Claude\'s program did not check, so the editor is unchanged.', true);
    } finally {
      btn.disabled = false;
    }
  }

  async store() {
    this.closePanel();
    this.panelFor = 'store';
    this.panel.hidden = false;
    this.panelSay('Storing…');
    const r = await this.tool('store', { text: this.text });
    const o = (r && r.structuredContent) || {};
    if (r && !r.isError) this.panelSay(`Stored${o.short_url || o.url ? `: ${o.short_url || o.url}` : '.'}`);
    else this.panelSay((r && resultText(r)) || 'Not stored.', true);
  }

  async libraryList() {
    const r = await this.tool('library_list', {});
    if (this.panelFor !== 'library') return;
    const list = this.panel.querySelector('ul');
    const items = (r && r.structuredContent && r.structuredContent.programs) || [];
    if (r && r.isError) this.panelSay(resultText(r) || 'The library is not available.', true);
    else if (!items.length) this.panelSay('Nothing saved yet.');
    list.replaceChildren(...items.map((it) => h('li', {}, h('button', { type: 'button', onclick: () => this.libraryGet(it.id) }, it.name || it.cid || it.id))));
  }

  /** library_get puts the program in the editor itself (onLoad). */
  async libraryGet(id) {
    const r = await this.tool('library_get', { id });
    if (r && !r.isError) this.closePanel();
    else this.panelSay((r && resultText(r)) || 'Not loaded.', true);
  }

  async librarySave() {
    const r = await this.tool('library_save', { text: this.text });
    if (r && !r.isError) { this.panelSay('Saved.'); this.libraryList(); } else this.panelSay((r && resultText(r)) || 'Not saved.', true);
  }

  async jam() {
    const r = await this.tool('jam_send', { text: this.text });
    this.say(r && !r.isError ? 'sent to the room' : (r && resultText(r)) || 'not sent');
  }

  /** A program from someone in the room: offered, not loaded over the text being edited. */
  jammed(msg) {
    const text = msg && msg.text;
    if (typeof text !== 'string') return;
    this.closePanel();
    this.panelFor = 'jam';
    this.panel.hidden = false;
    this.panel.replaceChildren(h('button', { type: 'button', class: 'tb', onclick: () => { this.load(text); this.closePanel(); } }, 'Load it'));
    const by = msg.by && typeof msg.by === 'object' ? msg.by.name : msg.by;
    this.panelSay(`${typeof by === 'string' && by ? by : 'Someone'} sent "${nameOf(text) || 'a program'}".`);
  }

  // ---- export ------------------------------------------------------------------------------------------------------

  /** A file offered as an <a download> link to a blob (no downloads capability here), clicked once. */
  offer(filename, data, type) {
    if (this.dlUrl) URL.revokeObjectURL(this.dlUrl);
    this.dlUrl = URL.createObjectURL(data instanceof Blob ? data : new Blob([data], { type }));
    this.dl.href = this.dlUrl;
    this.dl.download = filename;
    this.dl.textContent = `Download ${filename}`;
    this.dl.hidden = false;
    try { this.dl.click(); } catch { /* the link stays for the reader */ }
    return 'link';
  }

  /** With the downloads capability, the tools' export (the host confirms the save); else the link. */
  async exportAs(format) {
    try {
      if (this.caps.downloads) {
        const r = await this.tool('export', { format, text: this.text });
        this.say(r && !r.isError ? 'saved' : (r && resultText(r)) || 'not saved');
        return r && !r.isError ? 'downloads' : null;
      }
      if (format === 'btw') return this.offer(`${fileBase(this.text)}.btw`, this.text, 'text/plain');
      return this.offer(`${fileBase(this.text)}.wav`, await this.renderWav(), 'audio/wav');
    } catch (e) {
      this.say(String(e.message || e));
      return null;
    }
  }

  exportBtw() {
    return this.exportAs('btw');
  }

  exportWav() {
    return this.exportAs('wav');
  }

  // ---- sound -------------------------------------------------------------------------------------------------------

  /** One whole pass, offline: what the tools' render and the WAV export use. */
  async renderText(text) {
    const { module } = await loadModule();
    const { renderProgram } = await import('./beatscript-render.js');
    return renderProgram({ module, text, sampleRate: RENDER_RATE });
  }

  async renderWav() {
    this.say('rendering…');
    const r = await this.renderText(this.text);
    this.say('');
    this.lastRender = { frames: r.frames, bars: r.bars, seconds: r.seconds, cid: r.cid };
    this.renderPcm = r;
    return new Blob([wav(r.left, r.right, r.sampleRate)], { type: 'audio/wav' });
  }

  /** audio=render: render, then play the WAV in the <audio> (Web Audio where the page refuses blob: media). */
  async renderAndPlay() {
    if (this.busy) return;
    this.busy = true;
    // stop first: audio() then cancels the suspend stop() schedules, which would silence the Web Audio playback
    this.stop();
    const resumed = this.audio();
    try {
      await resumed;
      const blob = await this.renderWav();
      if (this.wavUrl) URL.revokeObjectURL(this.wavUrl);
      this.wavUrl = URL.createObjectURL(blob);
      if (await audioProbe(this.audioEl, this.wavUrl)) {
        this.audioEl.hidden = false;
        this.renderedVia = 'audio';
        await this.audioEl.play().catch(() => this.say('rendered: press play'));
      } else {
        const r = this.renderPcm;
        const buf = this.ctx.createBuffer(2, r.frames, r.sampleRate);
        buf.copyToChannel(r.left, 0);
        buf.copyToChannel(r.right, 1);
        this.bufSrc = this.ctx.createBufferSource();
        this.bufSrc.buffer = buf;
        this.bufSrc.connect(this.out);
        this.bufSrc.start();
        this.renderedVia = 'webaudio';
      }
    } catch (e) {
      this.say(String(e.message || e));
    } finally {
      this.busy = false;
    }
  }

  toggle() {
    if (this.busy) return;
    if (this.playing) this.stop();
    else this.start();
  }

  /** Ctrl/Cmd+Enter: playing, the edit lands live in the running program; else Play. */
  evaluate() {
    if (!this.playing) return this.start();
    const rep = this.check();
    if (!rep || !rep.ok) { this.say('fix the problems first'); return null; }
    const ev = this.run.eval(this.text);
    this.sections = rep.sections || [];
    this.say(ev && ev.ok ? '' : 'it does not play');
    return ev;
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
    return this.ctx.resume();
  }

  async start() {
    this.busy = true;
    // a render playing (audio=render) gives way to the live program
    this.stopRender();
    const resumed = this.audio();
    this.say('loading…');
    try {
      await resumed;
      const { module } = await loadModule();
      const rep = this.check();
      if (!rep || !rep.ok) throw new Error('fix the problems first');
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
      this.sections = rep.sections || [];
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

  tick() {
    if (!this.playing || !this.run || !this.engine) return;
    this.engine.apply(this.run.service(this.engine.grid()));
    const st = this.run.status();
    if (st && st.ended) return this.ended();
    this.paint();
  }

  /** The render's playback, silent. */
  stopRender() {
    if (this.bufSrc) { try { this.bufSrc.stop(); } catch { /* not started */ } this.bufSrc.disconnect(); this.bufSrc = null; }
    if (this.audioEl && !this.audioEl.paused) this.audioEl.pause();
  }

  /** Silent at once: the engine and what it had scheduled are let go, so the next Play starts clean from bar 1. */
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
    this.stopRender();
    if (this.ctx && this.ctx.state === 'running') {
      clearTimeout(this.suspendTimer);
      const ctx = this.ctx;
      this.suspendTimer = setTimeout(() => { if (!this.playing && ctx.state === 'running') ctx.suspend().catch(() => {}); }, SUSPEND_AFTER_MS);
    }
    if (was) this.dispatchEvent(new CustomEvent('stop'));
    if (this.btns) this.paint();
  }

  /** The play button and the section/bar line. */
  paint() {
    const st = this.run && this.playing ? this.run.status() : null;
    const m = positionModel(st, this.sections, this.playing);
    this.sec.textContent = m.sec;
    this.barLabel.textContent = m.bar;
    const b = this.btns.play;
    b.textContent = this.playing ? '■ Stop' : '▶ Play';
    b.classList.toggle('on', this.playing);
    b.setAttribute('aria-pressed', String(this.playing));
    b.setAttribute('aria-label', this.playing ? 'Stop' : 'Play (Ctrl+Enter)');
  }

  /** What the tests read. */
  get report() {
    const rep = this.rep;
    return {
      text: this.text,
      cid: rep ? rep.cid || null : null,
      ok: !!(rep && rep.ok),
      problems: this.problemList(),
      playing: this.playing,
      notes: this.notes.map((n) => ({ ...n })),
      tools: this.tools ? this.tools.list().filter((t) => t.available).map((t) => t.name) : [],
      buttons: this.btns ? Object.entries(this.btns).filter(([, b]) => !b.hidden).map(([k]) => k) : [],
      config: this.config,
      sec: this.sec && this.sec.textContent,
      bar: this.barLabel && this.barLabel.textContent,
      state: this.state && this.state.textContent,
      composer: this.engine ? this.engine.composer : null,
      installed: [...this.installed].sort(),
      highlighted: this.editor ? (this.editor.spansFn ? 'engine' : 'comments') : null,
      shared: this.shared || null,
      render: this.lastRender ? { ...this.lastRender, via: this.renderedVia || null } : null,
      set: this.set ? { at: this.setAt, titles: this.set.map(itemTitle) } : null,
    };
  }
}

if (typeof customElements !== 'undefined' && !customElements.get('beatscript-studio')) customElements.define('beatscript-studio', BeatscriptStudio);
