// The radio page (/radio, site/web/radio/index.html): a player. A station (a BeatScript program, station-model.js) and the
// tab's "up next" queue (queue-model.js, DESIGN.md "Up next") play through one transport and one playlist.
//
// - The station's answer (its canonical text, CID, the tracks from N and the segue after N) comes from the page wasm's
//   beatscript_station when the module has it, else POST /api/station.
// - The queue lives with this tab's hub session (POST /api/session, SSE events): MCP's `queue` tool writes it, and so does
//   the playlist (add by CID, link or example; remove; reorder), through POST /api/session/<id>/queue. Its items play
//   before the station goes on, and they advance only while someone is listening (the same presence rules: the server
//   decides every advance, POST …/queue/next).
// - What plays next is resolved while the current item plays (handoff-model.js): fetched, compiled into a run of its own,
//   or found rendered. A next item that is not ready at the boundary is bridged: a small loop at the outgoing tempo and key,
//   hot-reloaded so it lands on the next bar, until the next item comes in on a bar line. Never dead air.
// - Each row shows its render status (playlist-model.js RenderTracker: asked sparingly) and whether it plays from the
//   render or the live engine ("Play from": auto, render, live). The page starts a render only for the next row, only in
//   auto or render mode, and only while someone is listening.
// - Peek: the playing program's text in a drawer; an edit applies on the next bar (a hot reload), the row says "edited",
//   and "back to station" puts the track back. Closing it never stops playback.

import { h, shortCid, isLoopbackHost } from './util.js';
import { describeError, diagLine } from './errors.js';
import { makeApi, RENDER_VERSION } from './api.js';
import { prefGet, prefSet } from './prefs.js';
import { loadBeatscript } from './wasm.js';
import { createEngine } from './engine.js';
import { Driver, EngineSource } from './driver.js';
import { PresenceTracker, pauseReason, pauseMessage, playBars } from './queue-model.js';
import { SessionLink } from './session.js';
import { renderPlan, statusAt } from './render-model.js';
import {
  RenderTracker, playlistRows, playSource, renderLabel, fmtTime, nameOf, genreOf, tempoOf, keyOf, totalTime, renderToStart,
  moveItem, removeItem, parseAdd, PLAYBACK_MODES,
} from './playlist-model.js';
import { Handoff } from './handoff-model.js';
import {
  DEFAULT_STATION, KNOBS, FORMS, LENGTHS, SEEDED_GENRES, TEMPO_MIN, TEMPO_MAX,
  initialState, stationRequest, retuneRequest, applyAnswer, stepTo, advance, notch, toggleLock, stepRetuneValue,
  scanRequest, shareUrl, parseShare, fmtValue, maxN,
} from './station-model.js';

const TICK_MS = 25;
/** The station tracks the playlist shows after the one playing (the answer's count is this + 1). */
const AHEAD = 6;
const $ = (id) => document.getElementById(id);
const LABEL = { genre: 'Genre', seed: 'Seed', step: 'Step', forms: 'Forms', tempo: 'Tempo', key: 'Key', length: 'Length' };
const HELP = {
  genre: 'the generator\'s genre (retuned by a value only)',
  seed: 'the first track\'s seed: a whole number or a word; a notch is one up or down',
  step: 'from one track to the next: +1 counts the seed on, ~ draws it from the track before\'s CID',
  forms: 'the forms in turn, in this order: track N plays the N-th (mod how many); a notch rotates them',
  tempo: `${TEMPO_MIN} to ${TEMPO_MAX}; a notch is 2`,
  key: 'a notch is one step round the circle of fifths',
  length: 'short, standard or long',
};

class Radio {
  constructor() {
    this.api = makeApi();
    this.beatscript = null;
    this.ctx = null;
    this.engine = null;
    this.src = null;
    this.driver = null;
    this.timer = 0;
    this.starting = false;
    this.seq = 0;
    this.auto = 0;
    this.lastAdvanceAt = 0;
    this.state = initialState();
    this.ahead = []; // the station's tracks after N, from the last answer
    this.presence = new PresenceTracker({ doc: document, audio: () => (this.ctx ? this.ctx.state : 'none') });
    this.queue = null; // the queue's view (up_next, paused_reason, ...)
    this.tracker = new RenderTracker();
    this.meta = new Map(); // cid -> {title, genre, bpm, seconds, key, text, report}
    this.texts = new Map(); // cid -> text (fetched queue items)
    this.mode = readMode();
    this.failed = new Set(); // keys of items that failed to resolve (passed over)
    this.selected = null; // the playlist row chosen (its key)
    this.edited = null; // {original, text, cid}: the peek's edit of what plays
    this.listeners = [];
    this.audio = null;
    this.analyser = null;
    this.pos = {};
    this.handoff = new Handoff({
      provider: {
        upcoming: () => this.upcoming(),
        after: (key) => { this.failed.add(key); return this.upcoming(); },
        commit: (d) => this.commit(d),
      },
      sink: {
        play: (p, d) => this.soundPlay(p, d),
        bridge: (text, b) => this.soundBridge(text, b),
        stop: () => this.soundStop(),
        discard: (p) => { if (p && p.run) p.run.free(); },
      },
      verdict: () => this.verdict(),
      onChange: () => this.render(),
    });
    // the session link wants an app: what it calls, the radio answers
    this.upNext = {
      setView: (v) => this.setQueue(v),
      advance: (reason, item) => this.skip(item),
    };
  }

  // ---- what the browser tests read (the page before the player had these) ----------------------------------------------------

  get playing() { return this.isPlaying() && !this.handoff.paused; }
  get item() { const c = this.handoff.current; return c ? { kind: c.desc.kind, n: c.desc.n, key: c.desc.key, source: this.playingFrom } : null; }

  // ---- what SessionLink calls -----------------------------------------------------------------------------------------------

  on(fn) { this.listeners.push(fn); }
  emit(type) { for (const f of this.listeners) f(type); }
  get user() { return null; }
  isPlaying() { return !!this.handoff.current; }
  stop() { this.stopAll(); }
  toast(text) { this.plMsg(text); }
  showError(e) { this.plMsg(describeError(e).title, 'err'); }
  setProgram(text) { this.playText(text, 'from an assistant'); }
  async loadByCid(cid) { const t = await this.textOf({ cid }); this.playText(t, null, cid); }
  async loadExample(name) { const t = await this.api.exampleText(name); this.playText(t, name); }
  async openAny(url) { this.plMsg(`An assistant sent a link: add it to the queue to play it (${url.slice(0, 60)}).`); }
  async play() { if (!this.isPlaying()) await this.start(); }

  reportState() {
    const c = this.handoff.current;
    const d = c && c.desc;
    const f = this.driver && this.driver.last;
    const st = (f && f.status) || {};
    return {
      text: (d && d.text) || '',
      cid: (d && d.cid) || '',
      playing: this.isPlaying(),
      source: c ? (c.prepared.source === 'render' ? 'stream' : 'engine') : '',
      section: this.pos.section || '',
      bar: this.pos.songBar || 0,
      bars_in_section: this.pos.sectionBars || 0,
      phase: st.bar_in_phrase ?? 0,
      presence: this.presence.snapshot(),
      ts: new Date().toISOString(),
    };
  }

  async init() {
    const q = new URLSearchParams(location.search);
    // the JS stand-in for the module is for development on this machine only, as the editor's
    this.mock = q.get('mock') === '1' && isLoopbackHost(location.hostname);
    this.beatscriptP = loadBeatscript({ mock: this.mock }).then((b) => { this.beatscript = b; return b; }).catch((e) => {
      this.beatscriptError = e;
      console.warn('beatscript: the module did not load', e);
      return null;
    });
    this.pos = {};
    this.presence.attach(window);
    this.presence.onMediaKey = (a) => {
      if (a === 'play') this.start();
      else if (a === 'pause' || a === 'stop') this.stopAll();
      else if (a === 'nexttrack') this.skip();
      else if (a === 'previoustrack') this.prev();
    };
    const share = parseShare(q);
    let text = share.text || DEFAULT_STATION;
    if (share.cid) {
      this.wantCid = share.cid;
      try {
        text = (await this.api.getProgram(share.cid)).text;
      } catch (e) {
        this.msg(`The station ${shortCid(share.cid)} is not stored here (${describeError(e).title}): playing the default station.`, 'err');
        this.wantCid = null;
      }
    }
    this.state = initialState({ text, n: share.n });
    this.stationName();
    this.buildKnobs();
    this.wire();
    this.render();
    this.syncControls();
    // the tab's session: the queue (MCP's and the playlist's) lives with it
    this.session = new SessionLink(this);
    this.session.start();
    // a queue handed over from the editor's "Up next" (/radio?queue=<cid>,<cid>)
    const handed = (q.get('queue') || '').split(',').map((s) => s.trim()).filter(Boolean).slice(0, 20);
    if (handed.length) this.whenSession(() => this.queueWrite({ action: 'add', items: handed.map((cid) => ({ cid })) }, `Added ${handed.length} from the editor's queue.`));
    clearInterval(this.pollTimer);
    this.pollTimer = setInterval(() => this.pollRenders(), 500);
    await this.beatscriptP;
    const ok = await this.ask();
    if (ok && this.wantCid && this.state.cid !== this.wantCid) this.msg(`Careful: the link names ${shortCid(this.wantCid)}, but its text is the station ${shortCid(this.state.cid)}.`, 'err');
  }

  whenSession(fn) {
    if (this.session && this.session.id) return fn();
    const t = setInterval(() => { if (this.session && this.session.id) { clearInterval(t); fn(); } }, 200);
    setTimeout(() => clearInterval(t), 15000);
    return null;
  }

  // ---- the station's answer ------------------------------------------------------------------------------------------------

  /** The wasm's beatscript_station when the module has it, else the server's (the same JSON either way). */
  async call(req) {
    if (this.beatscript && this.beatscript.station) return this.beatscript.station(req);
    return this.api.station(req);
  }

  /**
   * Ask for the station at the state's N (and apply a retune, if `request` carries one): track N, the tracks after it the
   * playlist shows, and the segue after N. The latest ask wins. `restart`: what plays switches to the answer's track N
   * (a retune, a scan, a new position); otherwise the playing item plays on and only what is next is looked at again.
   */
  async ask({ request = null, restart = false } = {}) {
    const req = request || stationRequest(this.state, { count: AHEAD + 1 });
    if (!request) req.count = Math.max(req.count, Math.min(AHEAD + 1, maxN(this.state.knobs && this.state.knobs.step) - this.state.n + 1));
    else req.count = Math.min(AHEAD + 1, maxN((this.state.knobs && this.state.knobs.step)) - req.n + 1) || 1;
    const seq = ++this.seq;
    this.busy(true);
    let ans;
    try {
      ans = await this.call(req);
    } catch (e) {
      const d = describeError(e);
      ans = { ok: false, error: e && e.status === 404 ? 'this server does not answer for stations yet (POST /api/station)' : d.title, diagnostics: (e && e.details && e.details.diagnostics) || [] };
    }
    if (seq !== this.seq) return false;
    const { state, error } = applyAnswer(this.state, ans);
    this.state = state;
    this.busy(false);
    if (error) {
      this.msg(error.message, 'err', error.diagnostics);
    } else {
      this.ahead = (Array.isArray(ans.tracks) ? ans.tracks : []).filter((t) => t.n > this.state.n);
      for (const t of [this.state.now, ...this.ahead]) if (t && t.cid) this.learn(t.cid, t.text);
      this.updateUrl();
      this.syncControls();
      if (restart && this.isPlaying() && this.state.now) this.handoff.start(this.stationDesc(this.state.now));
      else this.handoff.refresh();
    }
    this.render();
    return !error;
  }

  busy(on) {
    this.waiting = on;
    for (const id of ['r-tune', 'r-scan', 'pl-like', 'pl-scan']) $(id).disabled = on || !this.state.knobs;
  }

  /** A retune ({knob: value | "+1" | "-1"}): refused here first when it turns a locked knob; N stays. */
  async tune(changes, what) {
    this.presence.touch();
    const r = retuneRequest(this.state, changes);
    if (!r.ok) return this.msg(r.error, 'err');
    const ok = await this.ask({ request: r.request, restart: true });
    const knobs = this.state.retuned && this.state.retuned.length ? this.state.retuned : Object.keys(changes);
    if (ok) this.msg(`${what || 'Retuned'}: ${knobs.map((k) => `${k} ${fmtValue(k, this.state.knobs[k])}`).join(', ')}. Still at position ${this.state.n}.`);
    return ok;
  }

  async scan() {
    this.presence.touch();
    const r = scanRequest(this.state);
    if (!r.ok) return this.msg(r.error, 'err');
    const ok = await this.ask({ request: r.request, restart: true });
    const text = `Scanned: ${r.pick.knob} a notch ${r.pick.dir === '+1' ? 'up' : 'down'}, to ${fmtValue(r.pick.knob, this.state.knobs[r.pick.knob])}. Still at position ${this.state.n}.`;
    if (ok) { this.msg(text); this.plMsg(text); }
  }

  /** More like this: the station next door along the seed, every other knob as it is (a lock on seed says no). */
  async moreLikeThis() {
    if (this.state.locks.includes('seed')) return this.plMsg('The seed is locked: unlock it for more like this.', 'err');
    const ok = await this.tune({ seed: '+1' }, 'More like this');
    if (ok) this.plMsg(`More like this: the station one seed along (seed ${fmtValue('seed', this.state.knobs.seed)}), the same genre, tempo, key and forms.`);
  }

  /** Tune: every knob whose control differs from the station's is retuned to its value. */
  tuneFromControls() {
    const k = this.state.knobs;
    if (!k) return;
    const want = this.readControls();
    if (want.error) return this.msg(want.error, 'err');
    const changes = {};
    for (const knob of KNOBS) {
      const v = want.values[knob];
      if (v == null) continue;
      const cur = k[knob];
      const differs = knob === 'forms' ? JSON.stringify(v) !== JSON.stringify(uniq(cur)) : String(v) !== String(cur);
      // an explicit step is `seed=+N` or `~`: a bare "+1" would be a notch (on a +3 station it toggles to ~)
      if (differs) changes[knob] = knob === 'step' ? stepRetuneValue(v) : v;
    }
    if (!Object.keys(changes).length) return this.msg('Nothing to tune: every knob is where the station has it.');
    return this.tune(changes, 'Tuned');
  }

  goto(n) {
    this.presence.touch();
    const s = stepTo(this.state, n);
    if (s === this.state) { this.syncPosition(); return null; }
    this.state = s;
    this.render();
    return this.ask({ restart: true });
  }

  updateUrl() {
    try { history.replaceState(null, '', this.href()); } catch { /* a sandboxed frame */ }
  }

  href() {
    return `${shareUrl(this.state)}${this.mock ? '&mock=1' : ''}`;
  }

  // ---- what plays next (handoff-model's provider) -------------------------------------------------------------------------

  /** The play order from here: the queue's items, else the segue after a station track, else the station's next track. */
  plan() {
    const out = [];
    const cur = this.handoff.current && this.handoff.current.desc;
    const q = this.queue && !this.queue.paused_reason ? this.queue.up_next || [] : [];
    for (const it of q) out.push(this.queueDesc(it));
    const seg = this.state.segue;
    if (!out.length && cur && cur.kind === 'track' && this.state.segues && seg && seg.from === cur.n && cur.n === this.state.n) out.push(this.segueDesc(seg));
    // the station goes on after the track it is at (or plays track N when nothing of it has played yet)
    // (a few of them: a track that fails to resolve is passed over for the one after it)
    const stationPlayed = cur && (cur.kind === 'track' || cur.kind === 'segue');
    const nextN = stationPlayed || this.stationStarted ? this.state.n + 1 : this.state.n;
    const pool = [this.state.now, this.state.next, ...this.ahead].filter(Boolean);
    for (let n = nextN; n < nextN + 3; n++) {
      const t = pool.find((x) => x.n === n);
      if (t) out.push(this.stationDesc(t));
    }
    return out;
  }

  upcoming() {
    return this.plan().find((d) => !this.failed.has(d.key)) || null;
  }

  learn(cid, text, extra = {}) {
    if (!cid) return;
    const m = this.meta.get(cid) || {};
    if (text && !m.text) {
      m.text = text;
      m.title = m.title || nameOf(text);
      m.genre = m.genre || genreOf(text);
      m.bpm = m.bpm || tempoOf(text);
      m.key = m.key || keyOf(text);
    }
    Object.assign(m, extra);
    this.meta.set(cid, m);
  }

  stationDesc(t) {
    const k = this.state.knobs || {};
    return this.desc({ key: `s:${t.n}`, kind: 'track', n: t.n, cid: t.cid, text: t.text, title: `${this.stationName()} #${t.n}${t.form && t.form !== 'seeded' ? ` · ${t.form}` : ''}`, genre: k.genre, tempo: k.tempo, keyName: k.key });
  }

  segueDesc(s) {
    const k = this.state.knobs || {};
    return this.desc({ key: `g:${s.from}`, kind: 'segue', n: s.from, cid: s.cid, text: s.text, title: `segue ${s.from} → ${s.to}${s.form ? ` · ${s.form}` : ''}`, genre: k.genre, tempo: k.tempo, keyName: k.key });
  }

  queueDesc(it, { pick = false } = {}) {
    const m = this.meta.get(it.cid) || {};
    return this.desc({ key: `q:${it.id}`, kind: 'queue', id: it.id, cid: it.cid, example: it.example, title: it.title || m.title || shortCid(it.cid || ''), item: it, pick, bars: it.bars, minutes: it.minutes });
  }

  /** A descriptor: what it is, and prepare(), which resolves it (the render when it plays from one, else a compiled run). */
  desc(d) {
    const self = this;
    d.prepare = async () => {
      const text = d.text || await self.textOf(d);
      d.text = text;
      self.learn(d.cid, text);
      const m = self.meta.get(d.cid) || {};
      d.tempo = d.tempo || m.bpm;
      d.musicKey = d.keyName || m.key;
      d.genre = d.genre || m.genre;
      const report = self.reportOf(d.cid, text);
      const render = d.cid ? self.tracker.get(d.cid) : { state: 'none' };
      const from = playSource(self.mode, render);
      if (from === 'render') return { source: 'render', url: render.url, report, text };
      if (from === 'wait') { await self.waitForRender(d); const r2 = self.tracker.get(d.cid); return { source: 'render', url: r2.url, report, text }; }
      await self.ready();
      const run = self.beatscript.newRun();
      const res = run.eval(text);
      if (!res || !res.ok) {
        run.free();
        const e = ((res && res.diagnostics) || []).find((x) => x.severity !== 'warning');
        throw new Error(`${d.title || 'the next item'} does not play here${e ? `: ${diagLine(e)}` : ''}`);
      }
      return { source: 'live', run, res, report, text };
    };
    return d;
  }

  /** A queue item's text: an example's by name, else the stored program's by CID (cached). */
  async textOf(d) {
    if (d.cid && this.texts.has(d.cid)) return this.texts.get(d.cid);
    let text;
    if (d.example) text = await this.api.exampleText(d.example);
    else text = (await this.api.getProgram(d.cid)).text;
    if (d.cid) this.texts.set(d.cid, text);
    return text;
  }

  /** The check report of a program (tempo, seconds, sections, loops), once per CID. */
  reportOf(cid, text) {
    const m = (cid && this.meta.get(cid)) || {};
    if (m.report) return m.report;
    let rep = null;
    try { rep = this.beatscript ? this.beatscript.check(text) : null; } catch { rep = null; }
    if (rep && cid) {
      const plan = renderPlan(rep);
      this.learn(cid, text, { report: rep, seconds: plan.seconds, bpm: rep.tempo || m.bpm, plan });
    }
    return rep;
  }

  /** Render mode: start the render (while someone listens) and wait for it; failed or too long: the item plays live. */
  waitForRender(d) {
    return new Promise((resolve, reject) => {
      const t0 = Date.now();
      const look = () => {
        const r = this.tracker.get(d.cid);
        if (r.state === 'ready' && r.url) return resolve();
        if (r.state === 'failed') return reject(new Error(`${d.title}: its render failed`));
        if (Date.now() - t0 > 5 * 60000) return reject(new Error(`${d.title}: its render took too long`));
        if (r.state === 'none' && this.listening()) this.startRender(d.cid, d.text);
        setTimeout(look, 1000);
      };
      look();
    });
  }

  /** An item starts: the station's N moves on; a queue item is taken from the queue (the server decides, by presence). */
  commit(d) {
    this.failed.delete(d.key);
    this.lastAdvanceAt = this.presence.now();
    if (d.kind === 'track') {
      this.stationStarted = true;
      if (d.n !== this.state.n) {
        this.state = d.n === this.state.n + 1 ? advance(this.state) : stepTo(this.state, d.n);
        this.updateUrl();
        this.ask();
      }
    } else if (d.kind === 'queue' && this.session && this.session.id) {
      const reason = d.pick ? 'pick' : 'ended';
      this.api.queueNext(this.session.id, { reason, presence: this.presence.snapshot(), item: d.id }).then((r) => {
        if (r && r.queue) this.setQueue(r.queue);
        if (r && !r.item && r.queue && r.queue.paused_reason) {
          this.halt();
          this.banner(pauseMessage(r.queue.paused_reason), true);
        }
      }).catch((e) => this.plMsg(`The queue did not answer: ${describeError(e).title}`, 'err'));
    }
    this.edited = null;
    this.render();
  }

  verdict() {
    const interacted = this.presence.lastInteraction > this.lastAdvanceAt;
    if (interacted) this.auto = 0;
    const why = pauseReason(this.presence.snapshot(), { auto: this.auto, interactedSinceAdvance: interacted });
    if (!why) this.auto++;
    return why;
  }

  listening() {
    return !pauseReason(this.presence.snapshot(), { auto: 0, interactedSinceAdvance: true });
  }

  // ---- the sound ------------------------------------------------------------------------------------------------------------

  /** The module and the engine, once: the engine only after a click (the browser's autoplay rule). */
  async ready() {
    await this.beatscriptP;
    if (!this.beatscript) throw new Error('the player did not load here: the station can be read but not played');
    if (!this.ctx) throw new Error('press play first');
    if (!this.analyser) {
      this.analyser = this.ctx.createAnalyser();
      this.analyser.fftSize = 256;
      this.analyser.smoothingTimeConstant = 0.7;
      this.analyser.connect(this.ctx.destination);
    }
    if (!this.engine) {
      const engine = this.mock
        ? await (await import('./wasm-mock.js')).createMockEngine({ context: this.ctx })
        : await createEngine({ context: this.ctx, wasmUrl: this.beatscript.wasmUrl, destination: this.analyser });
      await engine.ready;
      if (engine.on) {
        engine.on('error', (e) => { if (e && e.message) this.plMsg(e.message, 'err'); });
        // the worklet's own report keeps the runtime serviced while timers are throttled (a hidden tab)
        engine.on('grid', () => this.tick());
      }
      this.engine = engine;
      this.src = new EngineSource(engine);
      this.driver = new Driver(this.beatscript);
    }
  }

  audioEl() {
    if (!this.audio) {
      const a = new Audio();
      a.preload = 'auto';
      a.addEventListener('error', () => { if (this.playingFrom === 'render') this.plMsg('The render did not play here: going on live.', 'err'); });
      try { this.ctx.createMediaElementSource(a).connect(this.analyser); } catch { /* no Web Audio route: it plays on its own */ }
      this.audio = a;
    }
    return this.audio;
  }

  soundPlay(p, d) {
    this.lastBar = null;
    this.pos = {};
    this.startedAt = this.ctx ? this.ctx.currentTime : 0;
    if (p.source === 'render') {
      if (this.driver) this.driver.stopSound();
      const a = this.audioEl();
      a.src = p.url;
      a.currentTime = 0;
      a.play().catch((e) => this.plMsg(`The render did not start: ${e.message}`, 'err'));
      this.playingFrom = 'render';
    } else {
      if (this.audio) this.audio.pause();
      this.driver.adopt(p.run, p.text, p.res, this.src);
      p.run = null; // the driver has it now
      this.playingFrom = 'live';
    }
    const rep = p.report || {};
    this.target = d.kind === 'queue' ? playBars(d.item, rep) : playBars(null, rep);
    this.plan_ = renderPlan(rep);
    this.banner(null);
    this.render();
  }

  /** A bridge: hot-reloaded into the running program (it lands on the next bar), or started fresh after a render. */
  soundBridge(text) {
    if (this.audio && this.playingFrom === 'render') this.audio.pause();
    if (!this.driver) return;
    const res = this.driver.evaluate(text, this.src);
    if (!res || !res.ok) console.warn('beatscript radio: the bridge did not compile', res && res.diagnostics);
    this.playingFrom = 'live';
    this.target = null;
    this.startedAt = this.ctx ? this.ctx.currentTime : 0;
    this.render();
  }

  soundStop() {
    if (this.driver) this.driver.stopSound();
    if (this.audio) this.audio.pause();
    this.playingFrom = null;
    this.pos = {};
  }

  // ---- the transport ------------------------------------------------------------------------------------------------------

  toggle() {
    if (this.starting) return;
    if (this.isPlaying() && !this.handoff.paused) this.stopAll();
    else this.start();
  }

  async start(desc = null) {
    this.presence.touch();
    this.starting = true;
    // the context is made and resumed inside the click
    if (!this.ctx) this.ctx = new AudioContext({ latencyHint: 'playback' });
    const resumed = this.ctx.resume();
    this.banner('Loading the player…');
    try {
      await resumed;
      await this.ready();
      this.auto = 0;
      this.lastAdvanceAt = this.presence.now();
      clearInterval(this.timer);
      this.timer = setInterval(() => this.tick(), TICK_MS);
      // "Still listening?": the queue's pause is lifted by this interaction, and the next item plays
      const d = desc || this.resumeDesc();
      if (!d) { this.banner('Waiting for the station…'); return; }
      await this.handoff.start(d);
      this.banner(null);
    } catch (e) {
      this.banner(String((e && e.message) || e));
    } finally {
      this.starting = false;
      this.render();
    }
  }

  /** What Play plays: the item a pause stopped before, else the queue's next, else the station's track N. */
  resumeDesc() {
    if (this.handoff.paused && this.handoff.next && this.handoff.next.state !== 'failed') return this.handoff.next.desc;
    if (this.queue && (this.queue.up_next || []).length) return this.queueDesc(this.queue.up_next[0]);
    return this.state.now ? this.stationDesc(this.state.now) : null;
  }

  /** Next: what is next now (a ready one swaps at once; one still resolving is bridged to). */
  skip(item) {
    this.presence.touch();
    if (!this.isPlaying()) return;
    if (item && this.queue) {
      const it = (this.queue.up_next || []).find((x) => x.id === item || x.cid === item);
      if (it) return this.start(this.queueDesc(it, { pick: true }));
    }
    this.handoff.boundary();
  }

  prev() {
    this.presence.touch();
    const cur = this.handoff.current && this.handoff.current.desc;
    if (cur && cur.kind === 'track' && cur.n > 0) return this.goto(cur.n - 1);
    if (this.state.n > 0) return this.goto(this.state.n - 1);
    return null;
  }

  /** Stop the sound, keep the page's place. */
  halt() {
    clearInterval(this.timer);
    this.timer = 0;
    this.handoff.stop();
    this.lastBar = null;
  }

  stopAll() {
    this.halt();
    this.banner(null);
    this.render();
  }

  /** A program sent by an assistant (MCP eval or load): it plays now, a one-off; the queue and the station go on after it. */
  playText(text, title, cid = null) {
    const c = cid || (this.beatscript && this.beatscript.check(text) || {}).cid || null;
    this.learn(c, text);
    this.start(this.desc({ key: `x:${c || Date.now()}`, kind: 'one', cid: c, text, title: title || nameOf(text) || 'from an assistant' }));
  }

  tick() {
    if (!this.isPlaying()) return;
    const c = this.handoff.current;
    let bar = 0;
    let ended = false;
    if (this.playingFrom === 'render' && this.audio) {
      const t = this.audio.currentTime || 0;
      const plan = this.plan_ || { bpm: 120, bars: 16 };
      const sections = (c && c.prepared.report && c.prepared.report.sections) || [];
      const st = statusAt(t, plan, sections);
      bar = st.song_bar - 1;
      ended = this.audio.ended;
      this.pos = { elapsed: t, total: this.audio.duration || plan.seconds, section: st.section || '', sectionBar: st.section_bar, sectionBars: st.section_bars, songBar: st.song_bar };
    } else if (this.driver) {
      const f = this.driver.tick();
      if (!f) return;
      const st = f.status || {};
      bar = Math.max(0, st.bar || 0);
      ended = !!st.ended;
      const g = f.grid || {};
      const bpm = g.bpm || (c && c.desc.tempo) || (this.plan_ && this.plan_.bpm) || 120;
      // elapsed: the song's position (whole bars, and the beat within the bar from the clock), not the time since the
      // engine was asked: installing a program's nets takes a moment before its bar 0 sounds
      const songBar = st.song_bar ?? bar + 1;
      const inBar = Number.isFinite(g.beat) && g.beat > 0 ? (g.beat % 4) : 0;
      this.pos = { elapsed: Math.max(0, ((songBar - 1) * 4 + inBar) * 60 / bpm), total: this.handoff.bridging ? null : (this.plan_ && this.plan_.seconds), section: st.section || '', sectionBar: st.section_bar, sectionBars: st.section_bars, songBar: st.song_bar ?? bar + 1, bpm };
      this.marks = f.marks;
    }
    this.handoff.frame({ bar });
    if (!this.handoff.bridging && this.handoff.current) {
      if (ended || (this.target != null && bar >= this.target)) this.handoff.boundary();
    }
    if (this.lastBar !== bar) { this.lastBar = bar; this.renderNow(); }
    this.renderPos();
  }

  // ---- render statuses: asked sparingly; one render started, for the next row, while someone listens ------------------------------

  async pollRenders() {
    const now = Date.now();
    const rows = this.rows || [];
    // a station row's duration: its program checked here, one every 1.5 s, only for the rows that show
    if (this.beatscript && now - (this.lastMetaAt || 0) > 1500) {
      const r = rows.find((x) => x.cid && !(x.seconds > 0) && (this.meta.get(x.cid) || {}).text && !(this.meta.get(x.cid) || {}).report);
      if (r) { this.lastMetaAt = now; this.reportOf(r.cid, this.meta.get(r.cid).text); this.renderList(); }
    }
    this.tracker.want(rows.map((r) => r.cid).filter(Boolean));
    for (const cid of this.tracker.due(now, { hidden: document.visibilityState === 'hidden' })) {
      this.api.renderStatus(cid).then((answer) => this.tracker.settle(cid, Date.now(), { answer }), (error) => this.tracker.settle(cid, Date.now(), { error })).finally(() => this.renderList());
    }
    if (this.isPlaying()) {
      const r = renderToStart(rows, { mode: this.mode, listening: this.listening() });
      if (r) {
        // a queued program is stored (its CID is its URL); a station track is not, so its URL carries its text
        const m = this.meta.get(r.cid) || {};
        if (r.kind === 'queue') this.startRender(r.cid, null);
        else if (m.text) this.startRender(r.cid, m.text);
      }
    }
  }

  /**
   * Start a render on the server: GET the Opus stream and read it to its end (the server renders as it streams, and stops
   * when its last reader goes). A station track is not stored, so its URL carries its text (`<cid>.<packed>`).
   */
  async startRender(cid, text) {
    if (this.rendering && this.rendering.has(cid)) return;
    this.rendering = this.rendering || new Set();
    this.rendering.add(cid);
    this.tracker.mark(cid, { state: 'queued', mine: true });
    this.renderList();
    let url = this.api.renderUrl(cid, 'opus');
    try {
      if (text && this.beatscript) {
        const sh = this.beatscript.share(text);
        const packed = sh && sh.ok && typeof sh.url === 'string' ? (sh.url.split('#')[1] || '').split('~')[0].split('.').slice(1).join('.') : '';
        if (packed && sh.cid === cid) url = `/api/render/${cid}.${packed}?format=opus&v=${RENDER_VERSION}`;
      }
      const res = await fetch(url);
      if (!res.ok) throw Object.assign(new Error(`render: HTTP ${res.status}`), { status: res.status });
      this.tracker.mark(cid, { state: 'rendering', mine: true, seconds: Number(res.headers.get('X-Btw-Seconds')) || null });
      this.renderList();
      const reader = res.body.getReader();
      for (;;) { const { done } = await reader.read(); if (done) break; }
      this.tracker.mark(cid, { state: 'ready', mine: false, url: this.api.renderUrl(cid, 'opus') });
    } catch (e) {
      // a 429 (busy) is not a failure: it stays queued and is asked again later
      if (e && e.status === 429) this.tracker.mark(cid, { state: 'none', mine: false });
      else this.tracker.mark(cid, { state: 'failed', mine: false, error: String((e && e.message) || e) });
    } finally {
      this.rendering.delete(cid);
      this.renderList();
    }
  }

  // ---- the queue ------------------------------------------------------------------------------------------------------------

  setQueue(v) {
    if (!v || typeof v !== 'object') return;
    this.queue = v;
    for (const it of v.up_next || []) if (it.cid && !this.meta.has(it.cid)) this.fetchMeta(it);
    this.handoff.refresh();
    this.render();
  }

  /** What the playlist shows of a queued program: its title, genre, BPM and duration (its text, fetched once). */
  async fetchMeta(it) {
    this.meta.set(it.cid, this.meta.get(it.cid) || { title: it.title || '' });
    try {
      const text = await this.textOf(it);
      this.learn(it.cid, text);
      await this.beatscriptP;
      if (this.beatscript) this.reportOf(it.cid, text);
    } catch { /* the row shows what it has */ }
    this.renderList();
  }

  async queueWrite(body, done) {
    if (!this.session || !this.session.id) return this.plMsg('The tab has no session yet: try again in a moment.', 'err');
    this.presence.touch();
    try {
      const r = await fetch(`/api/session/${encodeURIComponent(this.session.id)}/queue`, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      const j = await r.json().catch(() => null);
      if (!r.ok) throw Object.assign(new Error((j && j.error && j.error.message) || (j && j.message) || `HTTP ${r.status}`), { status: r.status });
      if (j && j.queue) this.setQueue(j.queue);
      if (done) this.plMsg(done);
      return true;
    } catch (e) {
      this.plMsg(String(e.message || e), 'err');
      return false;
    }
  }

  /** Add: a CID, a /p/ link, a sealed share link (stored first, as share does) or an example name. */
  async add(input) {
    const p = parseAdd(input);
    if (p.error) return this.plMsg(p.error, 'err');
    if (p.example) return this.queueWrite({ action: 'add', items: [{ example: p.example }] }, `Queued the example ${p.example}.`);
    if (p.sealed) {
      await this.beatscriptP;
      const o = this.beatscript && this.beatscript.open(p.link);
      if (!o || !o.ok || o.verified !== true) return this.plMsg('That link does not open: its text is not the program its CID names.', 'err');
      try { await this.api.putProgram(o.cid, o.canonical); } catch (e) { if (e.status !== 200) return this.plMsg(`It could not be stored here: ${describeError(e).title}`, 'err'); }
      this.texts.set(o.cid, o.canonical);
      return this.queueWrite({ action: 'add', items: [{ cid: o.cid }] }, `Queued ${shortCid(o.cid)}.`);
    }
    return this.queueWrite({ action: 'add', items: [{ cid: p.cid }] }, `Queued ${shortCid(p.cid)}.`);
  }

  moveSelected(dir) {
    const rows = this.rows || [];
    const i = rows.findIndex((r) => r.key === this.selected);
    const r = rows[i];
    if (!r || r.kind !== 'queue') return this.plMsg('Choose a queued row to move: the station\'s own tracks keep their order.');
    const items = (this.queue && this.queue.up_next) || [];
    const at = items.findIndex((x) => x.id === r.id);
    const to = Math.max(0, Math.min(items.length - 1, at + dir));
    if (to === at) return null;
    this.queue = { ...this.queue, up_next: moveItem(items, r.id, to) }; // at once; the server's answer replaces it
    this.render();
    return this.queueWrite({ action: 'move', item: r.id, to });
  }

  removeSelected() {
    const r = (this.rows || []).find((x) => x.key === this.selected);
    if (!r || r.kind !== 'queue') return this.plMsg('Choose a queued row to remove: the station\'s own tracks are always there.');
    const rows = this.rows;
    const next = rows[rows.indexOf(r) + 1];
    this.selected = next ? next.key : null;
    this.queue = { ...this.queue, up_next: removeItem(this.queue.up_next, r.id) };
    this.render();
    return this.queueWrite({ action: 'remove', item: r.id });
  }

  /** Double-click or Enter: a queued row plays now (taken from the queue), a station row moves the station there. */
  playRow(key) {
    const r = (this.rows || []).find((x) => x.key === key);
    if (!r || r.current) return this.isPlaying() ? null : this.start();
    if (r.kind === 'queue') return this.start(this.queueDesc(r.item || (this.queue.up_next || []).find((x) => x.id === r.id), { pick: true }));
    if (r.kind === 'station') {
      const t = r.n === this.state.n ? this.state.now : this.ahead.find((x) => x.n === r.n);
      if (t) { this.stationStarted = true; return this.start(this.stationDesc(t)); }
      return this.goto(r.n);
    }
    return null;
  }

  // ---- peek: the playing program, edited on the next bar ----------------------------------------------------------------------

  togglePeek(open = $('peek').hidden) {
    $('peek').hidden = !open;
    $('p-peek').setAttribute('aria-expanded', String(open));
    if (open) this.fillPeek();
  }

  fillPeek() {
    const c = this.handoff.current;
    const text = this.edited ? this.edited.text : c ? c.desc.text || '' : (this.state.now && this.state.now.text) || '';
    const ta = $('peek-text');
    if (document.activeElement !== ta || !this.peekDirty) ta.value = text;
    this.peekDirty = false;
    this.peekFor = c ? c.desc.key : null;
    this.renderPeek();
  }

  applyPeek() {
    this.presence.touch();
    const c = this.handoff.current;
    if (!c || this.playingFrom !== 'live' || this.handoff.bridging) return this.peekMsg(this.playingFrom === 'render' ? 'This track plays from its render: switch "Play from" to live to edit it as it plays.' : 'Play first: an edit lands on the playing track.', 'err');
    const text = $('peek-text').value;
    const res = this.driver.evaluate(text, this.src);
    if (!res || !res.ok) {
      const d = ((res && res.diagnostics) || []).find((x) => x.severity !== 'warning');
      return this.peekMsg(d ? diagLine(d) : 'it does not check', 'err');
    }
    this.edited = { original: c.desc.text, text, cid: res.cid || null, key: c.desc.key };
    this.peekDirty = false;
    this.peekMsg(`Applied: lands on the next bar. Your version is ${shortCid(this.edited.cid || '')}.`);
    this.render();
    return true;
  }

  revertPeek() {
    const c = this.handoff.current;
    if (!c || !this.edited) return this.peekMsg('Nothing to put back: this is the track as it is.');
    this.driver.evaluate(this.edited.original, this.src);
    $('peek-text').value = this.edited.original;
    this.edited = null;
    this.peekMsg('Back to the station\'s track on the next bar.');
    this.render();
    return true;
  }

  async keepPeek() {
    if (!this.edited) return this.peekMsg('Edit and apply first: then this keeps your version.');
    const sh = this.beatscript && this.beatscript.share(this.edited.text);
    if (!sh || !sh.ok) return this.peekMsg('It could not be made a link.', 'err');
    try { await navigator.clipboard.writeText(sh.url); this.peekMsg('Kept: a link to your version is in the clipboard. It plays on until the track ends.'); } catch { this.peekMsg(`Kept: ${sh.url}`); }
    $('peek-open').href = sh.url;
    return true;
  }

  peekMsg(text, kind = '') {
    const el = $('peek-msg');
    el.textContent = text;
    el.className = `peek-msg${kind ? ` ${kind}` : ''}`;
  }

  renderPeek() {
    const c = this.handoff.current;
    const d = c && c.desc;
    $('peek-what').textContent = d ? `${d.title || ''}${this.edited ? ' · edited' : ''}` : 'nothing plays';
    if (d && d.text && this.beatscript) {
      try {
        const sh = this.beatscript.share((this.edited && this.edited.text) || d.text);
        if (sh && sh.ok) $('peek-open').href = sh.url;
      } catch { /* the editor link stays */ }
    }
  }

  // ---- the page ---------------------------------------------------------------------------------------------------------

  say(text) { this.banner(text); }

  banner(text, resume = false) {
    const el = $('p-banner');
    if (!text) { el.hidden = true; el.replaceChildren(); return; }
    el.hidden = false;
    el.replaceChildren(h('span', {}, text), ...(resume ? [h('button', { type: 'button', class: 'pbtn primary', onclick: () => this.start() }, 'Play')] : []));
  }

  msg(text, kind = '', diagnostics = []) {
    const el = $('r-msg');
    el.className = `msg${kind ? ` ${kind}` : ''}`;
    el.replaceChildren(h('span', {}, text), ...(diagnostics.length ? [h('pre', {}, diagnostics.map(diagLine).join('\n'))] : []));
  }

  plMsg(text, kind = '') {
    const el = $('pl-msg');
    el.className = `msg${kind ? ` ${kind}` : ''}`;
    el.textContent = text;
  }

  wire() {
    $('p-play').addEventListener('click', () => this.toggle());
    $('p-stop').addEventListener('click', () => this.stopAll());
    $('p-next').addEventListener('click', () => (this.isPlaying() ? this.skip() : this.goto(this.state.n + 1)));
    $('p-prev').addEventListener('click', () => this.prev());
    $('p-peek').addEventListener('click', () => this.togglePeek());
    $('peek-close').addEventListener('click', () => this.togglePeek(false));
    $('peek-apply').addEventListener('click', () => this.applyPeek());
    $('peek-revert').addEventListener('click', () => this.revertPeek());
    $('peek-keep').addEventListener('click', () => this.keepPeek());
    $('peek-text').addEventListener('input', () => { this.peekDirty = true; });
    $('peek-text').addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); this.applyPeek(); }
    });
    const mode = $('p-mode');
    mode.value = this.mode;
    mode.addEventListener('change', () => {
      this.mode = PLAYBACK_MODES.includes(mode.value) ? mode.value : 'auto';
      prefSet('radioMode', this.mode);
      // what is next is resolved again in the new mode
      this.handoff.next = null;
      this.handoff.refresh();
      this.render();
    });
    $('pl-like').addEventListener('click', () => this.moreLikeThis());
    $('pl-scan').addEventListener('click', () => this.scan());
    $('pl-add').addEventListener('submit', (e) => {
      e.preventDefault();
      const v = $('pl-add-in').value;
      // cleared when it is added, unless something else was typed meanwhile
      Promise.resolve(this.add(v)).then((ok) => { if (ok && $('pl-add-in').value === v) $('pl-add-in').value = ''; });
    });
    $('pl-up').addEventListener('click', () => this.moveSelected(-1));
    $('pl-down').addEventListener('click', () => this.moveSelected(1));
    $('pl-rm').addEventListener('click', () => this.removeSelected());
    $('pl-clear').addEventListener('click', () => this.queueWrite({ action: 'clear' }, 'The queue is clear: the station plays on.'));
    const list = $('pl-rows');
    list.addEventListener('click', (e) => {
      const li = e.target.closest('li[data-key]');
      if (li) { this.selected = li.dataset.key; this.renderList(); }
    });
    list.addEventListener('dblclick', (e) => {
      const li = e.target.closest('li[data-key]');
      if (li && !e.target.closest('a')) this.playRow(li.dataset.key);
    });
    list.addEventListener('keydown', (e) => this.listKey(e));
    // drag a queued row to reorder it
    list.addEventListener('dragstart', (e) => {
      const li = e.target.closest('li[data-key]');
      if (!li || !li.draggable) return;
      this.dragKey = li.dataset.key;
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', li.dataset.key);
    });
    list.addEventListener('dragover', (e) => { if (this.dragKey) { e.preventDefault(); e.dataTransfer.dropEffect = 'move'; } });
    list.addEventListener('drop', (e) => {
      e.preventDefault();
      const li = e.target.closest('li[data-key]');
      const from = (this.rows || []).find((r) => r.key === this.dragKey);
      this.dragKey = null;
      if (!li || !from || from.kind !== 'queue') return;
      const items = (this.queue && this.queue.up_next) || [];
      const over = (this.rows || []).find((r) => r.key === li.dataset.key);
      const to = over && over.kind === 'queue' ? items.findIndex((x) => x.id === over.id) : over && over.kind === 'station' ? items.length - 1 : 0;
      if (to < 0) return;
      this.queue = { ...this.queue, up_next: moveItem(items, from.id, to) };
      this.render();
      this.queueWrite({ action: 'move', item: from.id, to });
    });
    // the tuner and the program, as before
    $('r-n').addEventListener('change', (e) => this.goto(Number(e.target.value)));
    $('r-segues').addEventListener('change', (e) => {
      this.presence.touch();
      this.state = { ...this.state, segues: e.target.checked };
      this.ask();
    });
    $('r-tune').addEventListener('click', () => this.tuneFromControls());
    $('r-scan').addEventListener('click', () => this.scan());
    $('r-copy').addEventListener('click', async () => {
      const url = new URL(this.href(), location.href).href;
      try {
        await navigator.clipboard.writeText(url);
        this.msg('Copied the link: it opens this station at this position.');
      } catch {
        this.msg(`Copy this link: ${url}`);
      }
    });
    document.addEventListener('keydown', (e) => {
      if (e.target.closest && e.target.closest('input, textarea, select')) return;
      if (e.key === 'Escape' && this.isPlaying()) this.stopAll();
      else if (e.key === ' ' && !e.target.closest('button, a, [role="listbox"]')) { e.preventDefault(); this.toggle(); }
    });
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') this.pollRenders(); });
  }

  listKey(e) {
    const rows = this.rows || [];
    if (!rows.length) return;
    const i = Math.max(0, rows.findIndex((r) => r.key === this.selected));
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (e.altKey) { this.moveSelected(e.key === 'ArrowDown' ? 1 : -1); return; }
      const j = Math.max(0, Math.min(rows.length - 1, (this.selected == null ? -1 : i) + (e.key === 'ArrowDown' ? 1 : -1)));
      this.selected = rows[j].key;
      this.renderList();
      const li = $('pl-rows').querySelector(`li[data-key="${CSS.escape(this.selected)}"]`);
      if (li) li.scrollIntoView({ block: 'nearest' });
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (this.selected) this.playRow(this.selected);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      this.removeSelected();
    }
  }

  // ---- the knobs ---------------------------------------------------------------------------------------------------------------

  buildKnobs() {
    const box = $('r-knobs');
    this.ctl = {};
    this.nudges = {};
    this.locks = {};
    for (const knob of KNOBS) {
      const id = `k-${knob}`;
      let control;
      if (knob === 'genre') control = h('select', { id }, ...SEEDED_GENRES.map((g) => h('option', { value: g }, g)));
      else if (knob === 'length') control = h('select', { id }, ...LENGTHS.map((g) => h('option', { value: g }, g)));
      else if (knob === 'step') control = h('select', { id }, h('option', { value: '+1' }, '+1 (count on)'), h('option', { value: '~' }, '~ (from the track before)'));
      else if (knob === 'tempo') control = h('input', { id, type: 'number', min: String(TEMPO_MIN), max: String(TEMPO_MAX), step: '1', inputmode: 'numeric' });
      else if (knob === 'forms') control = h('div', { id, class: 'forms', role: 'group', 'aria-labelledby': `${id}-lbl` });
      else control = h('input', { id, type: 'text', autocomplete: 'off', spellcheck: 'false', placeholder: knob === 'key' ? 'am, c, f#m, d:dorian' : 'a number or a word' });
      this.ctl[knob] = control;
      const label = knob === 'forms' ? h('span', { class: 'k-name', id: `${id}-lbl` }, LABEL[knob]) : h('label', { class: 'k-name', for: id }, LABEL[knob]);
      const nudges = [];
      if (knob === 'step') nudges.push(this.nudgeBtn(knob, '+1', '⇄', 'Step: switch +1 and ~'));
      else if (knob !== 'genre') nudges.push(this.nudgeBtn(knob, '-1', '−', `${LABEL[knob]} one notch down`), this.nudgeBtn(knob, '+1', '+', `${LABEL[knob]} one notch up`));
      this.nudges[knob] = nudges;
      const lock = h('button', { type: 'button', class: 'tb tb-sm lock', 'aria-pressed': 'false', 'aria-label': `Lock ${LABEL[knob].toLowerCase()}`, title: `Lock ${LABEL[knob].toLowerCase()}: hold it fixed` }, h('span', { class: 'lock-ic', 'aria-hidden': 'true' }), h('span', {}, 'lock'));
      lock.addEventListener('click', () => this.toggleLockOf(knob));
      this.locks[knob] = lock;
      box.append(h('div', { class: 'knob', 'data-knob': knob },
        label,
        h('div', { class: 'k-ctl' }, control),
        h('div', { class: 'k-nudge' }, ...nudges),
        lock,
        h('p', { class: 'k-help' }, HELP[knob])));
    }
    // the playlist's own locks: the knobs that shape the family (genre, seed, step, forms)
    this.plLocks = {};
    for (const knob of ['genre', 'seed', 'step', 'forms', 'tempo', 'key']) {
      const b = h('button', { type: 'button', class: 'lk', 'aria-pressed': 'false', title: `Lock ${LABEL[knob].toLowerCase()}` }, LABEL[knob].toLowerCase());
      b.addEventListener('click', () => this.toggleLockOf(knob));
      this.plLocks[knob] = b;
      $('pl-locks').append(b);
    }
  }

  toggleLockOf(knob) {
    this.presence.touch();
    this.state = { ...this.state, locks: toggleLock(this.state.locks, knob) };
    this.syncControls();
  }

  nudgeBtn(knob, dir, glyph, label) {
    const b = h('button', { type: 'button', class: 'tb tb-sm nudge', 'data-dir': dir, 'aria-label': label }, glyph);
    b.addEventListener('click', () => this.tune({ [knob]: dir }, `${LABEL[knob]} a notch ${dir === '+1' ? 'up' : 'down'}`));
    return b;
  }

  /** The controls say the station's knobs (after an answer; never while someone types: only answers call this). */
  syncControls() {
    const k = this.state.knobs;
    const held = new Set(this.state.locks);
    for (const knob of KNOBS) {
      const c = this.ctl[knob];
      const v = k ? k[knob] : null;
      if (knob === 'forms') this.renderForms(v || ['seeded']);
      else if (knob === 'step') {
        const s = String(v || '+1');
        if (![...c.options].some((o) => o.value === s)) c.append(h('option', { value: s }, s));
        c.value = s;
      } else if (knob === 'genre' && v && ![...c.options].some((o) => o.value === v)) {
        c.append(h('option', { value: v }, v));
        c.value = v;
      } else c.value = v == null ? '' : String(v);
      if (knob === 'tempo' || knob === 'key') c.placeholder = v == null ? "the genre's" : '';
      if (knob === 'forms') for (const box of c.querySelectorAll('input')) box.disabled = held.has(knob);
      else c.disabled = held.has(knob);
      for (const b of this.nudges[knob]) {
        const to = k ? notch(knob, v, Number(b.dataset.dir)) : null;
        b.disabled = !k || held.has(knob) || to == null;
        b.title = held.has(knob) ? `${LABEL[knob]} is locked` : to == null ? 'no neighbour that way' : `to ${fmtValue(knob, to)}`;
      }
      const lock = this.locks[knob];
      lock.setAttribute('aria-pressed', String(held.has(knob)));
      lock.lastChild.textContent = held.has(knob) ? 'locked' : 'lock';
      if (this.plLocks[knob]) this.plLocks[knob].setAttribute('aria-pressed', String(held.has(knob)));
    }
    for (const id of ['r-tune', 'r-scan', 'pl-like', 'pl-scan']) $(id).disabled = !k || !!this.waiting;
  }

  /** The forms, as checkboxes: the station's rotation first, in its order, then the rest; Tune reads the checked ones in order. */
  renderForms(forms) {
    const box = this.ctl.forms;
    const order = [...uniq(forms), ...FORMS.filter((f) => !forms.includes(f))];
    box.replaceChildren(...order.map((f) => {
      const cb = h('input', { type: 'checkbox', value: f });
      cb.checked = forms.includes(f);
      return h('label', { class: 'check' }, cb, h('span', {}, f));
    }));
    if (uniq(forms).length !== forms.length) box.append(h('span', { class: 'k-note' }, `(the station plays ${forms.join(' ')}: a form twice is kept until forms are tuned)`));
  }

  readControls() {
    const v = {};
    for (const knob of KNOBS) {
      const c = this.ctl[knob];
      if (knob === 'forms') {
        v.forms = [...c.querySelectorAll('input:checked')].map((x) => x.value);
        if (!v.forms.length) return { error: 'Forms: check at least one.' };
      } else if (knob === 'tempo') {
        if (c.value === '') v.tempo = null;
        else {
          const t = Number(c.value);
          if (!Number.isFinite(t) || t < TEMPO_MIN || t > TEMPO_MAX) return { error: `Tempo: ${TEMPO_MIN} to ${TEMPO_MAX}.` };
          v.tempo = t;
        }
      } else {
        const s = String(c.value || '').trim();
        v[knob] = s === '' ? null : knob === 'key' ? s.toLowerCase() : s;
      }
    }
    return { values: v };
  }

  // ---- drawing ---------------------------------------------------------------------------------------------------------------

  stationName() {
    const n = nameOf(this.state.text);
    if (n) this.name = n;
    return this.name || 'station';
  }

  syncPosition() {
    const n = $('r-n');
    if (document.activeElement !== n || String(n.value) !== String(this.state.n)) n.value = String(this.state.n);
  }

  /** The rows the playlist shows now. */
  computeRows() {
    const c = this.handoff.current;
    const cur = c && c.desc;
    const plan = this.plan();
    const queue = plan.filter((d) => d.kind === 'queue').map((d) => d.item);
    // the station's rows: what comes after what plays (the segue is not a row of its own: the row after it says "via segue")
    const fromN = plan.find((d) => d.kind === 'track');
    const stationRows = [];
    if (fromN) {
      const all = [this.state.now, this.state.next, ...this.ahead].filter(Boolean);
      const seen = new Set();
      for (const t of all) if (t.n >= fromN.n && !seen.has(t.n) && stationRows.length < AHEAD) { seen.add(t.n); stationRows.push(t); }
    }
    const k = this.state.knobs || {};
    let now = null;
    if (cur) {
      const m = this.meta.get(cur.cid) || {};
      now = { key: cur.key, kind: cur.kind, n: cur.n, cid: this.edited ? this.edited.cid : cur.cid, id: cur.id,
        title: `${this.handoff.bridging ? '…bridging · ' : ''}${cur.title || m.title || ''}${this.edited ? ' · edited' : ''}`,
        genre: cur.genre || m.genre, bpm: cur.tempo || m.bpm, seconds: m.seconds, source: this.playingFrom === 'render' ? 'render' : 'live', url: c.prepared.url };
    }
    const nextKey = this.handoff.bridging && this.handoff.next ? this.handoff.next.desc.key : null;
    const rows = playlistRows({ now, queue, station: stationRows, stationInfo: { name: this.stationName(), genre: k.genre, tempo: k.tempo }, meta: this.meta, renders: this.tracker, mode: this.mode, bridging: nextKey });
    for (const r of rows) {
      if (r.kind === 'queue') r.item = queue.find((x) => `q:${x.id}` === r.key);
      if (this.failed.has(r.key)) r.failed = true;
      if (r.kind === 'station' && this.state.segues && cur && cur.kind === 'track' && r.n === cur.n + 1 && !queue.length) r.viaSegue = true;
      if (r.current) r.source = this.playingFrom === 'render' ? 'render' : 'live';
    }
    return rows;
  }

  render() {
    const s = this.state;
    this.syncPosition();
    $('r-segues').checked = s.segues;
    $('r-text').textContent = s.text;
    const cid = $('r-cid');
    cid.textContent = s.cid || 'not answered yet';
    cid.title = s.cid || '';
    $('r-link').href = this.href();
    const k = s.knobs || {};
    $('pl-family').textContent = s.knobs ? `${this.stationName()} · ${k.genre} · seed ${fmtValue('seed', k.seed)} · step ${k.step} · ${(k.forms || []).join(' ')}${k.tempo ? ` · ${k.tempo} bpm` : ''}${k.key ? ` · ${k.key}` : ''}` : '';
    this.renderNow();
    this.renderList();
    if (!$('peek').hidden && this.peekFor !== (this.handoff.current && this.handoff.current.desc.key)) this.fillPeek();
    else if (!$('peek').hidden) this.renderPeek();
    if (this.session) this.session.dirty = true;
  }

  /** The LED readout and the transport. */
  renderNow() {
    const c = this.handoff.current;
    const d = c && c.desc;
    const playing = this.isPlaying() && !this.handoff.paused;
    const play = $('p-play');
    play.disabled = this.starting || (!s0(this.state) && !playing && !(this.queue && (this.queue.up_next || []).length));
    play.classList.toggle('on', playing);
    play.setAttribute('aria-pressed', String(playing));
    play.setAttribute('aria-label', playing ? 'Pause' : this.handoff.paused ? 'Play on' : 'Play');
    $('p-state-ic').className = `lcd-state ${playing ? (this.handoff.bridging ? 'st-bridge' : 'st-play') : this.handoff.paused ? 'st-pause' : 'st-stop'}`;
    $('p-led').classList.toggle('on', playing);
    const title = d ? `${this.handoff.bridging ? `bridging to ${this.handoff.next ? this.handoff.next.desc.title : 'what is next'} · ` : ''}${d.title || ''}${this.edited ? ' (edited)' : ''}` : this.waiting ? 'Tuning in…' : s0(this.state) ? `${this.stationName()} #${this.state.n} · press play` : 'Tuning in…';
    const tEl = $('p-title');
    if (tEl.textContent !== title) {
      tEl.textContent = title;
      // a title wider than the readout scrolls (a CSS animation; still when the person prefers less motion)
      requestAnimationFrame(() => $('p-title-box').classList.toggle('long', tEl.scrollWidth > $('p-title-box').clientWidth + 2));
    }
    $('p-flag-src').textContent = this.handoff.bridging ? 'BRIDGE' : this.playingFrom === 'render' ? 'RENDER' : 'LIVE';
    $('p-flag-src').classList.toggle('lit', playing);
    $('p-flag-mode').textContent = this.mode.toUpperCase();
    $('p-prev').disabled = !(this.state.n > 0 || (d && d.n > 0));
    this.renderPos();
  }

  renderPos() {
    const p = this.pos || {};
    const c = this.handoff.current;
    const playing = this.isPlaying();
    $('p-elapsed').textContent = playing ? fmtTime(p.elapsed || 0) : '0:00';
    const total = playing ? p.total : (c && this.meta.get(c.desc.cid) || {}).seconds;
    $('p-total').textContent = fmtTime(total);
    $('p-section').textContent = playing && p.section ? `${p.section}${p.sectionBars ? ` ${p.sectionBar || 1}/${p.sectionBars}` : ''}` : playing && this.handoff.bridging ? 'bridge' : '—';
    $('p-bar').textContent = playing && p.songBar ? `bar ${p.songBar}` : '';
    const bpm = (c && c.desc.tempo) || (this.state.knobs && this.state.knobs.tempo);
    $('p-bpm').textContent = bpm ? `${Math.round(bpm)} bpm` : '';
    $('p-key').textContent = (c && c.desc.musicKey) || (this.state.knobs && this.state.knobs.key) || '';
    const frac = playing && p.total > 0 ? Math.max(0, Math.min(1, (p.elapsed || 0) / p.total)) : 0;
    $('p-seek').style.width = `${(frac * 100).toFixed(2)}%`;
    this.drawVis();
  }

  /** The level: a spectrum of what sounds (the analyser on the output), and the decks' roles lit as they fire. */
  drawVis() {
    const cv = $('p-vis');
    const g = cv.getContext && cv.getContext('2d');
    if (!g) return;
    const W = cv.width;
    const H = cv.height;
    g.clearRect(0, 0, W, H);
    const n = 20;
    const bw = Math.floor(W / n);
    let vals = new Array(n).fill(0);
    if (this.analyser && this.isPlaying()) {
      const buf = new Uint8Array(this.analyser.frequencyBinCount);
      this.analyser.getByteFrequencyData(buf);
      vals = vals.map((_, i) => {
        const a = Math.floor(((i / n) ** 1.6) * buf.length);
        const b = Math.max(a + 1, Math.floor((((i + 1) / n) ** 1.6) * buf.length));
        let m = 0;
        for (let j = a; j < b; j++) m = Math.max(m, buf[j]);
        return m / 255;
      });
    }
    for (let i = 0; i < n; i++) {
      const segs = Math.round(vals[i] * 10);
      for (let s = 0; s < 10; s++) {
        g.fillStyle = s < segs ? (s >= 8 ? '#ff6b81' : s >= 6 ? '#ffc850' : '#4cd06a') : 'rgba(76,208,106,0.10)';
        g.fillRect(i * bw + 1, H - (s + 1) * (H / 10) + 1, bw - 2, H / 10 - 2);
      }
    }
    // the roles that fired in the last beat (live only)
    const roles = $('p-roles');
    const lvl = {};
    for (const f of (this.playingFrom === 'live' && this.marks && this.marks.fires) || []) {
      const key = `${f.deck}:${f.role}`;
      lvl[key] = Math.max(lvl[key] || 0, Math.max(0, 1 - (f.age_beats || 0)));
    }
    this.roleSeen = this.roleSeen || new Map();
    for (const key of Object.keys(lvl)) if (!this.roleSeen.has(key) && this.roleSeen.size < 8) {
      const el = h('span', { class: 'role' }, key.replace(':', '·'));
      this.roleSeen.set(key, el);
      roles.append(el);
    }
    for (const [key, el] of this.roleSeen) el.classList.toggle('lit', (lvl[key] || 0) > 0.35);
  }

  renderList() {
    const rows = this.computeRows();
    this.rows = rows;
    if (this.selected && !rows.some((r) => r.key === this.selected)) this.selected = null;
    const list = $('pl-rows');
    const lis = rows.map((r, i) => {
      const st = r.render && r.render.state ? r.render.state : 'none';
      const label = r.bridging ? 'bridging' : r.failed ? 'skipped' : renderLabel(r.render);
      const cell = st === 'ready' && r.render.url ? h('a', { href: r.render.url, title: 'the Opus render', target: '_blank', rel: 'noopener' }, label) : h('span', {}, label);
      const from = r.source === 'render' ? 'R' : r.source === 'wait' ? 'R…' : 'L';
      const fromTitle = r.source === 'render' ? 'plays from its render' : r.source === 'wait' ? 'will play from its render once it is ready (a bridge plays meanwhile)' : 'plays live in this tab';
      const li = h('li', {
        class: `row k-${r.kind}${r.current ? ' cur' : ''}${r.key === this.selected ? ' sel' : ''}${r.bridging ? ' bridging' : ''}${r.failed ? ' failed' : ''}`,
        'data-key': r.key, role: 'option', 'aria-selected': String(r.key === this.selected), 'aria-current': r.current ? 'true' : null,
        draggable: r.kind === 'queue' ? 'true' : null, title: r.cid || '',
      },
      h('span', { class: 'c-n' }, `${i + 1}.`),
      h('span', { class: 'c-t' }, h('span', { class: 'tag' }, r.current ? '▶' : r.kind === 'queue' ? 'Q' : r.viaSegue ? '⤳' : ''), r.title || ''),
      h('span', { class: 'c-g' }, r.genre || ''),
      h('span', { class: 'c-b' }, r.bpm ? String(Math.round(r.bpm)) : ''),
      h('span', { class: 'c-d' }, fmtTime(r.seconds)),
      h('span', { class: `c-r st-${r.bridging ? 'bridging' : r.failed ? 'failed' : st}` }, h('i', { class: 'dot', 'aria-hidden': 'true' }), cell),
      h('span', { class: `c-s src-${r.source}`, title: fromTitle }, from));
      return li;
    });
    list.replaceChildren(...lis);
    const tt = totalTime(rows);
    const q = (this.queue && (this.queue.up_next || []).length) || 0;
    $('pl-total').textContent = `${rows.length} rows · ${q} queued · ${fmtTime(tt.seconds)}${tt.unknown ? ` + ${tt.unknown} unknown` : ''}`;
    const sel = rows.find((r) => r.key === this.selected);
    for (const id of ['pl-up', 'pl-down', 'pl-rm']) $(id).disabled = !sel || sel.kind !== 'queue';
    $('pl-clear').disabled = !q;
  }
}

// ---- small pieces ---------------------------------------------------------------------------------------------------------------

const uniq = (a) => [...new Set(a || [])];
const s0 = (state) => !!state.now;

function readMode() {
  const m = prefGet('radioMode', 'auto');
  return PLAYBACK_MODES.includes(m) ? m : 'auto';
}

const radio = new Radio();
window.__radio = radio; // for the browser tests
radio.init().catch((e) => {
  console.error('beatscript radio:', e);
  radio.banner(String((e && e.message) || e));
});
