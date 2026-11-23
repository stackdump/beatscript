// The render source: the program rendered offline in this browser, faster than real time (render.js, in a Worker, the
// same engine and worklet code the live engine plays, on a clock of its own), then played as one audio file
// (render-client.js: a blob: WAV on an <audio> element where the CSP allows it, else Web Audio on the same samples).
// No realtime engine runs while it plays, so a busy machine cannot make it underrun.
//
// Like the stream (stream-player.js) it is a clock for BeatscriptRun in virtual mode: grid() is the audio's own position
// (currentTime) and the program's tempo, so the follow view, the header and the markers light up the code as the
// engine's do. The page re-evaluates on a fresh run when the position goes backwards (a seek back, or the loop wrapping
// a program that does not loop itself): `epoch` changes, and the fresh run's first grid is bar 0, the next one the
// position, so the runtime counts the bars between (a forward jump) and paints where the audio is.
//
// Renders are cached by CID (RenderCache): a program whose CID has not changed plays again without rendering.

import { renderOnce, playable } from './render-client.js';
import { renderPlan, RENDER_LIMITS } from './render-model.js';

/** Renders kept: by count and by bytes of WAV (a 600 s render is ~100 MB). */
const CACHE_ENTRIES = 4;
const CACHE_BYTES = 256 * 1024 * 1024;
/** A step back in the audio's position bigger than this is a wrap or a seek, not jitter. */
const BACK_EPS = 0.25;

/** Rendered programs by CID, least recently used dropped first; an entry is the render's samples (not its transport). */
export class RenderCache {
  constructor({ entries = CACHE_ENTRIES, bytes = CACHE_BYTES } = {}) {
    this.max = entries;
    this.maxBytes = bytes;
    this.map = new Map();
  }
  get(cid) {
    const e = this.map.get(cid);
    if (!e) return null;
    this.map.delete(cid);
    this.map.set(cid, e);
    return e;
  }
  has(cid) {
    return this.map.has(cid);
  }
  put(cid, entry) {
    this.map.delete(cid);
    this.map.set(cid, entry);
    let bytes = 0;
    for (const e of this.map.values()) bytes += e.frames * 4;
    for (const [k, e] of this.map) {
      if (this.map.size <= 1 || (this.map.size <= this.max && bytes <= this.maxBytes)) break;
      this.map.delete(k);
      bytes -= e.frames * 4;
    }
  }
  get size() {
    return this.map.size;
  }
}

export class RenderPlayer {
  /**
   * context/destination: the page's AudioContext and level node (Web Audio playback); module: the compiled
   * WebAssembly.Module; text and report: the program and its check; cache: a RenderCache; volume: 0..1 for the <audio>
   * element (which does not go through the page's level node); render: renderOnce (injected for tests).
   */
  constructor({ context, destination, module, text, report, cache = null, volume = 1, onChange = () => {}, render = renderOnce, element = true }) {
    this.kind = 'render';
    this.ctx = context;
    this.dest = destination || (context && context.destination);
    this.module = module;
    this.text = text;
    this.report = report;
    this.cid = report && report.cid;
    this.cache = cache;
    this.volume = volume;
    this.onChange = onChange;
    this.renderFn = render;
    this.element = element;
    this.bpm = (report && report.tempo) || 120;
    this.plan = renderPlan(report, RENDER_LIMITS);
    this.loops = this.plan.loops; // the program loops itself: the runtime goes round with the audio
    this.progress = { fraction: 0, bars: 0, totalBars: this.plan.bars };
    this.pb = null; // render-client.js playable
    this.rendered = null; // {seconds, frames, bars, ms, via, playback, cached}
    this.error = null;
    this.stopped = false;
    this.paused = false;
    this.endedFlag = false;
    this.playing = { A: false, B: false };
    this.base = 0; // seconds of loops gone round (a looping program's beat goes on counting)
    this.lastT = 0;
    this.epoch = 0;
    this.anchor = false; // the next grid is bar 0 (a fresh run after a step back)
    this._abort = typeof AbortController !== 'undefined' ? new AbortController() : null;
    this._lastState = '';
  }

  /**
   * Render (or take the cached render of this CID), then play it from the start; with `play: false` it is left ready
   * (the page stops what was playing first, then calls play(), so two never sound at once).
   */
  async start({ play = true } = {}) {
    let r = this.cache && this.cid ? this.cache.get(this.cid) : null;
    const cached = !!r;
    let via = 'cache';
    if (!r) {
      this._emit();
      try {
        const out = await this.renderFn(this.module, this.text, {
          signal: this._abort ? this._abort.signal : null,
          onProgress: (p) => { this.progress = { fraction: p.fraction, bars: p.bars, totalBars: p.totalBars }; this.onChange('progress'); },
        });
        r = out.result;
        via = out.via;
      } catch (e) {
        if (this.stopped) return;
        this.error = e;
        this._emit();
        throw e;
      }
      if (this.stopped) return;
      if (this.cache && this.cid) this.cache.put(this.cid, r);
    }
    this.progress = { fraction: 1, bars: r.bars, totalBars: r.bars };
    this.pb = await playable(r, { ctx: () => this.ctx, out: () => this.dest, element: this.element });
    if (this.stopped) { this.pb.release(); this.pb = null; return; }
    const tr = this.pb.transport;
    tr.loop = this.loops;
    tr.volume = this.volume;
    tr.onended = () => { this.endedFlag = true; this._emit(); };
    this.rendered = { seconds: r.seconds, frames: r.frames, bars: r.bars, ms: r.ms, via, playback: this.pb.playback, cached, notes: r.notes || [] };
    if (this.plan.bpm) this.bpm = this.plan.bpm;
    if (play) await this.play();
    else { this.paused = true; this._emit(); }
  }

  /** Start the rendered audio (after start({play: false})). A browser that wants a fresh gesture leaves it paused. */
  async play() {
    const tr = this.transport;
    if (!tr || this.stopped) return;
    this.paused = false;
    try {
      await tr.play();
    } catch (e) {
      this.paused = true;
      this._emit();
      throw e;
    }
    this._emit();
  }

  get transport() {
    return this.pb ? this.pb.transport : null;
  }

  /** Echo BeatscriptRun's commands into the playing flags (virtual mode: the commands themselves are discarded). */
  echo(cmds) {
    for (const c of cmds || []) if (c && c.op === 'play') this.playing[c.deck] = !!c.on;
  }

  /** Seconds into the render that are sounding now. */
  mediaTime() {
    const tr = this.transport;
    return tr ? tr.time : 0;
  }

  get duration() {
    return this.rendered ? this.rendered.seconds : this.plan.seconds;
  }

  /** The clock BeatscriptRun runs on: beat = (loops gone round + the audio's position) x tempo / 60; null until it sounds. */
  grid() {
    const tr = this.transport;
    if (!tr || this.stopped || !(this.playing.A || this.playing.B)) return null;
    const t = tr.time;
    if (t < this.lastT - BACK_EPS) {
      // the audio went back: a looping program goes round with it, anything else starts a fresh run at bar 0
      if (this.loops && this.lastT > this.duration - 1) this.base += this.duration;
      else { this.lastT = t; this.restart(); return null; }
    }
    this.lastT = t;
    if (this.anchor) {
      this.anchor = false;
      return { deck: 'A', beat: 0, bpm: this.bpm, playing: { ...this.playing } };
    }
    return { deck: 'A', beat: ((this.base + t) * this.bpm) / 60, bpm: this.bpm, playing: { ...this.playing } };
  }

  /** The page evaluates the program again on a fresh run (it watches `epoch`); the next grid is that run's bar 0. */
  restart() {
    this.epoch++;
    this.base = 0;
    this.anchor = true;
    this.playing = { A: false, B: false };
    this.endedFlag = false;
  }

  /** Seconds of audio ready ahead of the position: all of it, once rendered. */
  buffered() {
    return this.rendered ? Math.max(0, this.duration - this.mediaTime()) : 0;
  }

  /** 'rendering' | 'playing' | 'paused' | 'ended' | 'error' | 'stopped' */
  get state() {
    if (this.stopped) return 'stopped';
    if (this.error) return 'error';
    if (!this.pb) return 'rendering';
    if (this.endedFlag) return 'ended';
    if (this.paused) return 'paused';
    return 'playing';
  }

  pause() {
    const tr = this.transport;
    if (!tr || this.paused || this.stopped) return;
    tr.pause();
    this.paused = true;
    this._emit();
  }

  async resume() {
    const tr = this.transport;
    if (!tr || !this.paused || this.stopped) return;
    this.paused = false;
    if (this.endedFlag) { this.endedFlag = false; tr.seek(0); }
    await tr.play();
    this._emit();
  }

  /** To `t` seconds: forward, the runtime counts the bars between; back, a fresh run (restart). */
  seek(t) {
    const tr = this.transport;
    if (!tr || this.stopped) return;
    const to = Math.max(0, Math.min(t, this.duration));
    if (to < tr.time - BACK_EPS) this.restart();
    tr.seek(to);
    this.lastT = to;
    if (this.endedFlag) { this.endedFlag = false; if (!this.paused) tr.play(); }
    this._emit();
  }

  setLoop(on) {
    this.loopOn = !!on;
    if (this.transport) this.transport.loop = this.loopOn;
    this._emit();
  }

  get loop() {
    return this.transport ? this.transport.loop : this.loops;
  }

  setVolume(v) {
    this.volume = v;
    if (this.transport) this.transport.volume = v;
  }

  _emit() {
    const s = this.state;
    if (s !== this._lastState) {
      this._lastState = s;
      this.onChange(s);
    }
  }

  /** Silent at once: the audio stops, a render under way is dropped (its worker with it). The cache keeps the samples. */
  stop() {
    if (this.stopped) return;
    this.stopped = true;
    if (this._abort) this._abort.abort();
    if (this.pb) { this.pb.release(); this.pb = null; }
    this._emit();
  }
}

/** The VirtualSource of a render: the Driver's source; `detached` when the page swaps in a fresh run on the same player. */
export class RenderSource {
  constructor(player) {
    this.kind = 'render';
    this.clock = player;
    this.detached = false;
  }
  grid() {
    return this.clock.grid();
  }
  handle(cmds) {
    if (!this.detached) this.clock.echo(cmds);
  }
  stop() {
    if (!this.detached) this.clock.stop();
  }
}
