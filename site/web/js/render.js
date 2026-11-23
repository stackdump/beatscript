// render.js: one pass of a BeatScript program rendered offline, faster than real time, by the engine the
// live player plays it on (js/engine.js and its worklet, js/engine-worklet.js), with no AudioContext.
//
//   import { renderProgram } from './beatscript-render.js';
//   const r = await renderProgram({ module, text, sampleRate: 44100, onProgress });
//   // r: {left, right, sampleRate, frames, bars, bpm, seconds, loops, notes, cid, lead, ms}
//
// How. createEngine is handed a context of our own: its audio node is the worklet's processor itself, constructed in
// this realm (a Worker, the page or Node), joined to engine.js by a pair of in-process ports. The clock is the frames
// rendered so far: `currentTime` and `sampleRate`, which the processor reads as an AudioWorkletGlobalScope's globals,
// are defined for the length of the render. The loop is the live page's, on that clock: every STEP frames the runtime
// is serviced with the engine's grid and its commands applied (as web/parity.html does in an OfflineAudioContext); in
// between, the processor's process() is called a render quantum (128 frames) at a time. Before a step goes on,
// everything in flight lands: the ports' messages, and the compile of any net a command loaded (the composer runs in
// this realm). Nothing waits on a wall clock, so the same program at the same rate is the same PCM, every time.
//
// The frames before the first deck starts (its net installing) are not kept: the render begins on the program's first
// downbeat, and is exactly renderPlan's bars long (render-model.js: one pass; a looping song once; capped at 256 bars
// and 600 s, as the site's /api/render).
//
// `module` is the compiled WebAssembly.Module of wasm/beatscript_bg.wasm. One render at a time in a realm (they share the
// clock globals): a second waits for the first.

import { createEngine } from './engine.js';
import * as glue from '../wasm/beatscript.js';
import { renderPlan, framesOf, RENDER_LIMITS } from './render-model.js';

const QUANTUM = 128;
/** Frames between two services of the runtime (~23 ms at 44.1 kHz; the live page's loop is 25 ms, parity.html's 1024). */
const STEP = 1024;
/** The most audio rendered and dropped while the first deck's net installs before the render starts anyway. */
const MAX_LEAD_SECONDS = 10;
/** How often onProgress is called, and (on a page) the thread is given back, by the wall clock: it changes no sample. */
const PROGRESS_MS = 50;

// Realm-wide, not module-wide: two copies of this file in one realm (the editor's and a player package's, or a test that
// loads both) share the AudioWorkletGlobalScope shim the first one installs, its registry, and the one-at-a-time queue
// (every render defines the same clock globals).
const shared = globalThis[Symbol.for('beatscript.render')] || (globalThis[Symbol.for('beatscript.render')] = { processors: new Map(), nextPort: null, queue: Promise.resolve() });
const processors = shared.processors;

/** The AudioWorkletGlobalScope names the worklet module uses at its top level, for this realm, once. */
function shimScope() {
  if (typeof globalThis.AudioWorkletProcessor !== 'function') {
    globalThis.AudioWorkletProcessor = class AudioWorkletProcessor {
      constructor() {
        this.port = shared.nextPort;
      }
    };
  }
  if (typeof globalThis.registerProcessor !== 'function') {
    globalThis.registerProcessor = (name, cls) => processors.set(name, cls);
  }
}

/** A macrotask: every microtask queued before it (port deliveries, the engine's batched sends) has run. */
const macrotask = typeof setImmediate === 'function'
  ? () => new Promise((r) => setImmediate(r))
  : (() => {
    let ch = null;
    const waiting = [];
    return () => new Promise((r) => {
      if (!ch) {
        ch = new MessageChannel();
        ch.port1.onmessage = () => { const f = waiting.shift(); if (f) f(); };
      }
      waiting.push(r);
      ch.port2.postMessage(0);
    });
  })();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());

/** Two ends of an in-process MessagePort: a message is delivered in a microtask, in order. */
function portPair() {
  const end = () => ({ onmessage: null, peer: null, closed: false, postMessage(data) { const to = this.peer; queueMicrotask(() => { if (!to.closed && to.onmessage) to.onmessage({ data }); }); }, close() { this.closed = true; } });
  const a = end();
  const b = end();
  a.peer = b;
  b.peer = a;
  return [a, b];
}

/** The context createEngine is given: the processor in this realm, on the clock `clock.frame`. */
function offlineContext(sampleRate, clock, made) {
  return {
    sampleRate,
    get currentTime() { return clock.frame / sampleRate; },
    destination: {},
    audioWorklet: {
      async addModule(url) {
        shimScope();
        await import(String(url));
      },
    },
    createGain() {
      return { gain: { value: 1, setTargetAtTime() {} }, connect() {}, disconnect() {} };
    },
    createBeatscriptNode(name, options) {
      const Cls = processors.get(name);
      if (!Cls) throw new Error(`render: the worklet did not register ${name}`);
      const [pagePort, procPort] = portPair();
      shared.nextPort = procPort;
      let proc;
      try { proc = new Cls(options); } finally { shared.nextPort = null; }
      made.processor = proc;
      made.port = procPort;
      return { port: pagePort, connect() {}, disconnect() { pagePort.close(); } };
    },
  };
}

/** Define the worklet scope's clock globals for the render; returns the undo. */
function clockGlobals(sampleRate, clock) {
  const had = {};
  for (const k of ['sampleRate', 'currentTime']) had[k] = Object.getOwnPropertyDescriptor(globalThis, k);
  Object.defineProperty(globalThis, 'sampleRate', { configurable: true, get: () => sampleRate });
  Object.defineProperty(globalThis, 'currentTime', { configurable: true, get: () => clock.frame / sampleRate });
  return () => {
    for (const k of ['sampleRate', 'currentTime']) {
      if (had[k]) Object.defineProperty(globalThis, k, had[k]);
      else delete globalThis[k];
    }
  };
}

/**
 * Render one pass of `text`. Options: module (WebAssembly.Module, required), sampleRate (44100), limits
 * ({maxBars, maxSeconds}: 256 and 600), onProgress({frames, total, bars, totalBars, fraction}), signal (an
 * AbortSignal), yieldMs (give the thread back this often by the wall clock: a page's render; default 50).
 */
export function renderProgram(opts) {
  const run = shared.queue.then(() => render(opts));
  shared.queue = run.catch(() => {});
  return run;
}

async function render({ module, text, sampleRate = 44100, limits = RENDER_LIMITS, onProgress = null, signal = null, yieldMs = PROGRESS_MS } = {}) {
  if (!module) throw new Error('render: a compiled WebAssembly.Module is needed');
  const t0 = now();
  glue.initSync({ module });
  const report = JSON.parse(glue.beatscript_check(text));
  if (!report.ok) {
    const d = (report.diagnostics || []).find((x) => x.severity === 'error');
    throw new Error(d ? `line ${d.line}: ${d.message}` : 'the program does not check');
  }
  if (report.uses_assets) throw new Error('this program uses samples or loops, which the bundled engine has no files for');
  const plan = renderPlan(report, limits);
  const total = framesOf(plan.bars, plan.bpm, sampleRate);
  const barFrames = ((240 / plan.bpm) * sampleRate);

  const clock = { frame: 0 };
  const undo = clockGlobals(sampleRate, clock);
  const made = {};
  let engine = null;
  let rt = null;
  try {
    const context = offlineContext(sampleRate, clock, made);
    engine = await createEngine({ context, module, composer: 'main' });
    const errors = [];
    engine.on('error', (e) => errors.push(e.message));
    const proc = made.processor;
    if (!proc || !proc.dj) throw new Error('render: the engine did not start');

    rt = new glue.BeatscriptRun();
    const ev = JSON.parse(rt.eval(text));
    if (!ev.ok) throw new Error('the program does not play');

    // everything in flight lands: the ports' messages, then any compile the commands started, then what it sent
    const settle = async () => {
      await macrotask();
      for (let i = 0; engine.preparing > 0 && i < 60000; i++) await sleep(0);
      await macrotask();
    };
    const service = async () => {
      await settle();
      const cmds = JSON.parse(rt.service(JSON.stringify(engine.grid())));
      if (Array.isArray(cmds) && cmds.length) {
        engine.apply(cmds);
        await settle();
      }
    };
    const playing = () => {
      for (const d of ['A', 'B']) {
        try { if (JSON.parse(proc.dj.state(d)).playing) return true; } catch { /* no deck */ }
      }
      return false;
    };

    const left = new Float32Array(total);
    const right = new Float32Array(total);
    const outL = new Float32Array(QUANTUM);
    const outR = new Float32Array(QUANTUM);
    const outputs = [[outL, outR]];
    const inputs = [];
    let kept = 0;
    let lead = 0;
    let started = false;
    let since = 0;
    let lastYield = now();
    const maxLead = Math.round(MAX_LEAD_SECONDS * sampleRate);
    const progress = () => {
      if (onProgress) {
        try { onProgress({ frames: kept, total, bars: Math.min(plan.bars, Math.floor(kept / barFrames + 1e-9)), totalBars: plan.bars, fraction: kept / total }); } catch { /* the caller's */ }
      }
    };

    // the first service: the program starts at once, as when a page evaluates it
    await service();
    while (kept < total) {
      if (signal && signal.aborted) throw Object.assign(new Error('the render was stopped'), { name: 'AbortError' });
      if (since >= STEP) {
        since = 0;
        await service();
        if (proc.failed) throw new Error(errors[errors.length - 1] || 'the engine stopped');
        if (now() - lastYield >= yieldMs) {
          progress();
          // on a page, a timer (not a message) so the page can paint between slices
          await sleep(0);
          lastYield = now();
        }
      }
      if (!started && (playing() || lead >= maxLead)) started = true;
      outL.fill(0);
      outR.fill(0);
      proc.process(inputs, outputs);
      clock.frame += QUANTUM;
      since += QUANTUM;
      if (!started) { lead += QUANTUM; continue; }
      const n = Math.min(QUANTUM, total - kept);
      left.set(n === QUANTUM ? outL : outL.subarray(0, n), kept);
      right.set(n === QUANTUM ? outR : outR.subarray(0, n), kept);
      kept += n;
    }
    progress();
    return { left, right, sampleRate, frames: total, bars: plan.bars, programBars: plan.programBars, bpm: plan.bpm, seconds: total / sampleRate, loops: plan.loops, cut: plan.cut, notes: plan.notes, cid: report.cid, sections: report.sections || [], lead, errors, ms: now() - t0 };
  } finally {
    try { if (engine) engine.dispose(); } catch { /* gone */ }
    // the processor's 'dispose' frees its Dj; deliver it, then nothing more
    await macrotask();
    if (made.processor && made.processor.dj) { try { made.processor.dj.free(); } catch { /* freed */ } made.processor.dj = null; }
    try { if (rt) rt.free(); } catch { /* freed */ }
    undo();
  }
}
