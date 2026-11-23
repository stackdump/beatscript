// render-client.js: the render path a page uses, shared by the editor's "render" source (render-player.js) and
// <beatscript-track mode="render"> (site/player): render.js in a Worker when the browser allows one from where this file
// is, else on the page's thread in slices; the result as a 16-bit WAV in memory; played through an <audio> element on
// a blob: URL where the page's CSP allows blob: media, else through Web Audio on the same samples. Both playbacks have
// one interface (a transport): play(), pause(), seek(t), time, duration, loop, paused, onended, dispose().
//
//   const { result, via } = await renderOnce(module, text, { onProgress, signal });
//   const pb = await playable(result, { ctx: () => audioContext, out: () => destinationNode });
//   // pb: {transport, playback: 'audio' | 'webaudio', blob, url, release()}

import { RENDER_LIMITS, wav } from './render-model.js';

/** The render's sample rate (btwd render's default). */
export const RENDER_RATE = 44100;
/** How long a blob: WAV may take to load before Web Audio plays it instead. */
export const BLOB_PROBE_MS = 4000;

// ---- the render, in a Worker when the browser allows one from where this file is, else here

let workerState = null; // null untried | Promise | {worker, next, jobs} | false (none: render here)

function startWorker() {
  return (async () => {
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
      const s = { worker: w, next: 1, jobs: new Map() };
      w.onmessage = (e) => {
        const m = e.data;
        const job = m && s.jobs.get(m.id);
        if (!job) return;
        if (m.type === 'progress') { if (job.onProgress) job.onProgress(m); }
        else { s.jobs.delete(m.id); if (m.type === 'done') job.resolve(m.result); else job.reject(new Error(m.error)); }
      };
      return s;
    } catch {
      return false;
    }
  })();
}

function renderWorker() {
  if (workerState === null) workerState = startWorker().then((s) => (workerState = s));
  return Promise.resolve(workerState);
}

/** A stopped render's worker is let go (it would otherwise go on rendering to the end): the next render starts another. */
function dropWorker(s) {
  if (workerState !== s) return;
  workerState = null;
  try { s.worker.terminate(); } catch { /* gone */ }
  for (const j of s.jobs.values()) j.reject(Object.assign(new Error('the render was stopped'), { name: 'AbortError' }));
  s.jobs.clear();
}

const aborted = () => Object.assign(new Error('the render was stopped'), { name: 'AbortError' });

/**
 * Render `text` once: {result, via} (via 'worker' | 'page'). result is render.js renderProgram's. Options: onProgress,
 * signal (an AbortSignal: a stop drops the render at once, the worker included), sampleRate, limits, worker (false:
 * render on the page's thread).
 */
export async function renderOnce(module, text, { onProgress = null, signal = null, sampleRate = RENDER_RATE, limits = RENDER_LIMITS, worker = true } = {}) {
  if (signal && signal.aborted) throw aborted();
  const ws = worker ? await renderWorker() : false;
  if (ws) {
    const id = ws.next++;
    const p = new Promise((resolve, reject) => ws.jobs.set(id, { resolve, reject, onProgress }));
    ws.worker.postMessage({ type: 'render', id, module, text, sampleRate, limits });
    if (signal) signal.addEventListener('abort', () => { if (ws.jobs.has(id)) dropWorker(ws); }, { once: true });
    return { result: await p, via: 'worker' };
  }
  const { renderProgram } = await import('./render.js');
  return { result: await renderProgram({ module, text, sampleRate, limits, onProgress, signal }), via: 'page' };
}

// ---- the render's playback: an <audio> on a blob: WAV, or Web Audio on the same samples

/** An <audio> element playing `url`, or null when the page will not load it (a CSP without blob: in media-src). */
export function audioPlayer(url, probeMs = BLOB_PROBE_MS) {
  return new Promise((resolve) => {
    const a = new Audio();
    a.preload = 'auto';
    let done = false;
    const end = (ok) => { if (done) return; done = true; clearTimeout(t); a.onloadedmetadata = a.onerror = null; resolve(ok ? a : null); };
    const t = setTimeout(() => end(false), probeMs);
    a.onloadedmetadata = () => end(Number.isFinite(a.duration) && a.duration > 0);
    a.onerror = () => end(false);
    a.src = url;
  });
}

/** The transport of an <audio> element. */
export function elementTransport(a) {
  const tr = {
    kind: 'audio',
    el: a,
    get time() { return a.currentTime; },
    get duration() { return a.duration; },
    get paused() { return a.paused; },
    set loop(v) { a.loop = !!v; },
    get loop() { return a.loop; },
    set volume(v) { a.volume = Math.max(0, Math.min(1, v)); },
    onended: null,
    play: () => a.play(),
    pause: () => a.pause(),
    seek: (t) => { a.currentTime = Math.max(0, Math.min(t, a.duration || 0)); },
    dispose: () => { a.pause(); a.removeAttribute('src'); a.load(); },
  };
  a.addEventListener('ended', () => { if (tr.onended) tr.onended(); });
  return tr;
}

/** The transport of an AudioBuffer on an AudioContext (getCtx) into a node (getOut). */
export function bufferTransport(getCtx, getOut, buffer) {
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
    set volume(_v) { /* the destination node carries the level */ },
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

/**
 * A render made playable: its WAV in memory (a Blob and its blob: URL), and a transport on it, an <audio> where the
 * page allows blob: media, else Web Audio on `ctx()` into `out()`. {transport, playback, blob, url, release()}:
 * release() disposes the transport and revokes the URL.
 */
export async function playable(r, { ctx, out, probeMs = BLOB_PROBE_MS, element = true } = {}) {
  const bytes = wav(r.left, r.right, r.sampleRate);
  const blob = new Blob([bytes], { type: 'audio/wav' });
  const url = URL.createObjectURL(blob);
  const a = element ? await audioPlayer(url, probeMs) : null;
  let transport;
  let playback = 'audio';
  if (a) {
    transport = elementTransport(a);
  } else {
    // no blob: media here: the same samples through Web Audio
    playback = 'webaudio';
    const c = ctx();
    const buf = c.createBuffer(2, r.frames, r.sampleRate);
    buf.copyToChannel(r.left, 0);
    buf.copyToChannel(r.right, 1);
    transport = bufferTransport(ctx, out, buf);
  }
  return {
    transport,
    playback,
    blob,
    url,
    release() {
      try { transport.dispose(); } catch { /* gone */ }
      try { URL.revokeObjectURL(url); } catch { /* revoked */ }
    },
  };
}
