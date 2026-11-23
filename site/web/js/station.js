// The radio page (/radio, site/web/radio/index.html): a station (a BeatScript program, station-model.js) tuned and played
// in the browser. The station's answer (its canonical text, CID, the tracks at N and N+1 and the segue between) comes
// from the page wasm's beatscript_station when the module has it, else from POST /api/station. Playback is the embed player's:
// the module and the engine (after the first click), a fresh run per track, a 25 ms service loop, and the engine's own
// grid reports while the tab is hidden. A track plays one pass (queue-model.js passBars/playBars) or to its end; then
// the segue (when segues are on) and track N+1, and N moves on, only while someone is listening (PresenceTracker). Nothing
// is stored: a track is played from its text.

import { h, shortCid, isLoopbackHost } from './util.js';
import { describeError, diagLine } from './errors.js';
import { makeApi } from './api.js';
import { loadBeatscript } from './wasm.js';
import { createEngine } from './engine.js';
import { Driver, EngineSource } from './driver.js';
import { PresenceTracker, pauseReason, pauseMessage, playBars } from './queue-model.js';
import {
  DEFAULT_STATION, KNOBS, FORMS, LENGTHS, SEEDED_GENRES, TEMPO_MIN, TEMPO_MAX,
  initialState, stationRequest, retuneRequest, applyAnswer, stepTo, advance, notch, toggleLock, stepRetuneValue,
  scanRequest, shareUrl, parseShare, fmtValue, maxN,
} from './station-model.js';

const TICK_MS = 25;
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
    this.playing = false;
    this.starting = false;
    this.item = null; // what plays: {kind: 'track' | 'segue', n, text, cid, target}
    this.pending = null; // after a pause: 'advance' or the segue item to play on Play
    this.seq = 0;
    this.auto = 0;
    this.lastAdvanceAt = 0;
    this.state = initialState();
    this.presence = new PresenceTracker({ doc: document, audio: () => (this.ctx ? this.ctx.state : 'none') });
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
    this.presence.attach(window);
    this.presence.onMediaKey = (a) => {
      if (a === 'play') this.start();
      else if (a === 'pause' || a === 'stop') this.stop();
      else if (a === 'nexttrack') this.goto(this.state.n + 1);
      else if (a === 'previoustrack') this.goto(this.state.n - 1);
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
    this.buildKnobs();
    this.wire();
    this.render();
    this.syncControls();
    await this.beatscriptP;
    const ok = await this.ask();
    if (ok && this.wantCid && this.state.cid !== this.wantCid) this.msg(`Careful: the link names ${shortCid(this.wantCid)}, but its text is the station ${shortCid(this.state.cid)}.`, 'err');
  }

  // ---- the station's answer ------------------------------------------------------------------------------------------------

  /** The wasm's beatscript_station when the module has it, else the server's (the same JSON either way). */
  async call(req) {
    if (this.beatscript && this.beatscript.station) return this.beatscript.station(req);
    return this.api.station(req);
  }

  /**
   * Ask for the station at the state's N (and apply a retune, if `request` carries one). The latest ask wins. `restart`:
   * what plays switches to the answer's track N at once (a retune, a scan, a new position); otherwise (up next asked for
   * after an advance) the playing track plays on.
   */
  async ask({ request = null, restart = false } = {}) {
    const req = request || stationRequest(this.state);
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
      // a request made while the station plays (up next, a retune, a new position) that fails stops it, and says why
      if (this.playing || this.pending) {
        this.halt();
        this.pending = null;
        this.say(`Stopped: ${error.message}`);
      }
    } else {
      this.updateUrl();
      this.syncControls();
      if (restart && this.playing && this.state.now) this.playItem(trackItem(this.state.now));
    }
    this.render();
    return !error;
  }

  busy(on) {
    this.waiting = on;
    if (on && !this.playing) this.say('Tuning in…');
    else if (!on && !this.playing && !this.pending) this.say(this.state.now ? 'Press play to listen.' : '');
    for (const id of ['r-tune', 'r-scan']) $(id).disabled = on || !this.state.knobs;
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
    if (ok) this.msg(`Scanned: ${r.pick.knob} a notch ${r.pick.dir === '+1' ? 'up' : 'down'}, to ${fmtValue(r.pick.knob, this.state.knobs[r.pick.knob])}. Still at position ${this.state.n}.`);
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

  // ---- playback --------------------------------------------------------------------------------------------------------------

  toggle() {
    if (this.starting) return;
    if (this.playing) this.stop();
    else this.start();
  }

  /** The module and the engine, once: the engine only after a click (the browser's autoplay rule). */
  async ready() {
    await this.beatscriptP;
    if (!this.beatscript) throw new Error('the player did not load here: the station can be read but not played');
    if (!this.engine) {
      const engine = this.mock
        ? await (await import('./wasm-mock.js')).createMockEngine({ context: this.ctx })
        : await createEngine({ context: this.ctx, wasmUrl: this.beatscript.wasmUrl });
      await engine.ready;
      if (engine.on) {
        engine.on('error', (e) => { if (e && e.message) this.say(e.message); });
        // the worklet's own report keeps the runtime serviced while timers are throttled (a hidden tab)
        engine.on('grid', () => this.tick());
      }
      this.engine = engine;
      this.src = new EngineSource(engine);
      this.driver = new Driver(this.beatscript);
    }
  }

  async start() {
    this.presence.touch();
    this.starting = true;
    // the context is made and resumed inside the click
    if (!this.ctx) this.ctx = new AudioContext({ latencyHint: 'playback' });
    const resumed = this.ctx.resume();
    this.say('Loading the player…');
    try {
      await resumed;
      await this.ready();
      this.playing = true;
      this.auto = 0;
      this.lastAdvanceAt = this.presence.now();
      clearInterval(this.timer);
      this.timer = setInterval(() => this.tick(), TICK_MS);
      const p = this.pending;
      this.pending = null;
      if (p === 'advance') await this.advanceTrack();
      else if (p) this.playItem(p);
      else if (this.state.now) this.playItem(trackItem(this.state.now));
      else this.say('Waiting for the station…');
    } catch (e) {
      this.playing = false;
      this.say(String((e && e.message) || e));
    } finally {
      this.starting = false;
      this.render();
    }
  }

  /** A fresh run for one item (a track or a segue): its own program, from bar 1. */
  playItem(item) {
    if (!this.driver || !item) return false;
    this.driver.stopSound();
    const res = this.driver.evaluate(item.text, this.src);
    if (!res || !res.ok) {
      const d = ((res && res.diagnostics) || []).find((x) => x.severity !== 'warning');
      this.stop();
      this.say(`${item.kind === 'segue' ? `The segue from track ${item.n}` : `Track ${item.n}`} does not play here${d ? `: ${diagLine(d)}` : ''}.`);
      return false;
    }
    // one pass of a looping program, else until it ends (the program's own check: its song bars, its sections)
    const report = this.beatscript.check(item.text);
    this.item = { ...item, target: playBars(null, report) };
    this.render();
    return true;
  }

  tick() {
    if (!this.playing || !this.driver || !this.item) return;
    const t = this.driver.tick();
    if (!t) return;
    const st = t.status || {};
    if (st.ended || (this.item.target != null && (st.bar || 0) >= this.item.target)) {
      this.boundary();
      return;
    }
    const bar = Math.max(0, st.bar || 0) + 1;
    if (bar !== this.lastBar) {
      this.lastBar = bar;
      this.say(`${this.item.kind === 'segue' ? `Segue to track ${this.item.n + 1}` : `Track ${this.item.n}`} · bar ${bar}${this.item.target ? ` of ${this.item.target}` : ''}`);
    }
  }

  /** The playing item is over: the segue after a track (segues on), else track N+1; only while someone is listening. */
  boundary() {
    const cur = this.item;
    this.item = null;
    const seg = this.state.segue;
    const next = cur.kind === 'track' && this.state.segues && seg && seg.from === cur.n && cur.n === this.state.n ? segueItem(seg) : 'advance';
    const interacted = this.presence.lastInteraction > this.lastAdvanceAt;
    if (interacted) this.auto = 0;
    const why = pauseReason(this.presence.snapshot(), { auto: this.auto, interactedSinceAdvance: interacted });
    if (why) {
      this.halt();
      this.pending = next;
      this.say(pauseMessage(why));
      this.render();
      return;
    }
    this.lastAdvanceAt = this.presence.now();
    if (next === 'advance') {
      this.auto++;
      this.advanceTrack();
    } else this.playItem(next);
  }

  /** N moves on: track N+1 (up next, already answered) plays, and what is up next after it is asked for. */
  async advanceTrack() {
    if (this.state.n >= maxN(this.state.knobs && this.state.knobs.step)) { this.stop(); this.say('The end of the station: it reaches no further.'); return; }
    this.state = advance(this.state);
    this.updateUrl();
    this.render();
    const now = this.state.now;
    if (now) this.playItem(trackItem(now));
    else if (this.driver) this.driver.stopSound(); // up next is not answered yet: silence until it is
    const ok = await this.ask();
    if (!now && ok && this.playing && this.state.now) this.playItem(trackItem(this.state.now));
  }

  /** Stop the sound (the run puts back what it held), keep the page's place. */
  halt() {
    clearInterval(this.timer);
    this.timer = 0;
    if (this.driver) this.driver.stopSound();
    this.item = null;
    this.playing = false;
    this.lastBar = null;
  }

  stop() {
    this.halt();
    this.pending = null;
    this.say('Stopped.');
    this.render();
  }

  say(text) {
    $('r-state').textContent = text;
  }

  msg(text, kind = '', diagnostics = []) {
    const el = $('r-msg');
    el.className = `msg${kind ? ` ${kind}` : ''}`;
    el.replaceChildren(h('span', {}, text), ...(diagnostics.length ? [h('pre', {}, diagnostics.map(diagLine).join('\n'))] : []));
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
      lock.addEventListener('click', () => {
        this.presence.touch();
        this.state = { ...this.state, locks: toggleLock(this.state.locks, knob) };
        this.syncControls();
      });
      this.locks[knob] = lock;
      box.append(h('div', { class: 'knob', 'data-knob': knob },
        label,
        h('div', { class: 'k-ctl' }, control),
        h('div', { class: 'k-nudge' }, ...nudges),
        lock,
        h('p', { class: 'k-help' }, HELP[knob])));
    }
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
    }
    for (const id of ['r-tune', 'r-scan']) $(id).disabled = !k || !!this.waiting;
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

  // ---- the page ------------------------------------------------------------------------------------------------------------------

  wire() {
    $('r-play').addEventListener('click', () => this.toggle());
    $('r-prev').addEventListener('click', () => this.goto(this.state.n - 1));
    $('r-next').addEventListener('click', () => this.goto(this.state.n + 1));
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
      if (e.key === 'Escape' && this.playing) this.stop();
    });
  }

  syncPosition() {
    const n = $('r-n');
    if (document.activeElement !== n || String(n.value) !== String(this.state.n)) n.value = String(this.state.n);
    $('r-prev').disabled = this.state.n <= 0;
    $('r-next').disabled = this.state.n >= maxN(this.state.knobs && this.state.knobs.step);
  }

  render() {
    const s = this.state;
    this.syncPosition();
    $('r-segues').checked = s.segues;
    const play = $('r-play');
    play.disabled = this.starting || (!s.now && !this.playing);
    play.classList.toggle('on', this.playing);
    play.setAttribute('aria-pressed', String(this.playing));
    $('r-play-lbl').textContent = this.playing ? 'Stop' : this.pending ? 'Play on' : 'Play';
    // now playing: what sounds (a segue between N and N+1 is still "after track N")
    $('r-now-dl').replaceChildren(...facts(s.now, s.n));
    $('r-now').classList.toggle('live', this.playing && !!this.item && this.item.kind === 'track');
    $('r-up-dl').replaceChildren(...facts(s.next, s.n + 1));
    const seg = $('r-segue');
    if (s.segues && s.segue) {
      seg.replaceChildren(h('b', {}, this.playing && this.item && this.item.kind === 'segue' ? 'Playing the segue: ' : 'First, the segue: '), `${s.segue.form || 'cross'} of track ${s.segue.from} into ${s.segue.to}${s.segue.phrase ? `, sung ${s.segue.phrase}` : ''} · `, cidCode(s.segue.cid));
      seg.hidden = false;
    } else {
      seg.replaceChildren(s.segues ? '' : 'Segues are off: track to track.');
      seg.hidden = s.segues;
    }
    $('r-text').textContent = s.text;
    const cid = $('r-cid');
    cid.textContent = s.cid || 'not answered yet';
    cid.title = s.cid || '';
    $('r-link').href = this.href();
  }
}

// ---- small pieces ---------------------------------------------------------------------------------------------------------------

const uniq = (a) => [...new Set(a || [])];

const trackItem = (t) => ({ kind: 'track', n: t.n, text: t.text, cid: t.cid });
const segueItem = (s) => ({ kind: 'segue', n: s.from, text: s.text, cid: s.cid });

function cidCode(cid) {
  return cid ? h('code', { title: cid }, shortCid(cid, 8, 6)) : h('span', { class: 'dim' }, 'no CID');
}

/** A track's card: position, form, seed, theme, phrase, CID and its text (read-only). */
function facts(t, n) {
  if (!t) return [h('dt', {}, 'Position'), h('dd', {}, String(n)), h('dt', {}, 'Track'), h('dd', { class: 'dim' }, 'asking…')];
  const out = [];
  const row = (k, ...v) => out.push(h('dt', {}, k), h('dd', {}, ...v));
  row('Position', String(t.n));
  row('Form', t.form || '');
  row('Seed', String(t.seed ?? ''), t.theme != null && String(t.theme) !== String(t.seed) ? h('span', { class: 'dim' }, ` · theme ${t.theme}`) : '');
  if (t.phrase) row('Phrase', t.phrase);
  row('CID', cidCode(t.cid));
  out.push(h('dt', { class: 'sr' }, 'Text'), h('dd', { class: 'wide' }, h('details', {}, h('summary', {}, 'its program'), h('pre', { class: 'src' }, t.text || ''))));
  return out;
}

const radio = new Radio();
window.__radio = radio; // for the browser tests
radio.init().catch((e) => {
  console.error('beatscript radio:', e);
  radio.say(String((e && e.message) || e));
});
