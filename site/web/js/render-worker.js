// render-worker.js: render.js in a dedicated Worker, so a long render never holds the page's thread. Shared by the
// editor's render source and <beatscript-track mode="render">: render-client.js starts it when this file is on the page's
// own origin (a Worker's script must be: from a CDN the browser refuses it, and the page renders on its own thread
// instead, in slices).
//
//   page -> worker  {type:'render', id, module, text, sampleRate, limits}
//   worker -> page  {type:'progress', id, ...}  {type:'done', id, result}  (left/right transferred)  {type:'failed', id, error}

import { renderProgram } from './render.js';

self.onmessage = async (e) => {
  const m = e.data;
  if (!m || m.type !== 'render') return;
  try {
    const r = await renderProgram({
      module: m.module,
      text: m.text,
      sampleRate: m.sampleRate,
      limits: m.limits,
      // nothing else runs on this thread: give it back only to post the progress
      yieldMs: 100,
      onProgress: (p) => self.postMessage({ type: 'progress', id: m.id, ...p }),
    });
    self.postMessage({ type: 'done', id: m.id, result: r }, [r.left.buffer, r.right.buffer]);
  } catch (err) {
    self.postMessage({ type: 'failed', id: m.id, error: String((err && err.message) || err) });
  }
};
self.postMessage({ type: 'ready' });
