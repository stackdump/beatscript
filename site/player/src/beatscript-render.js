// beatscript-render.js: the package's `./render` export. The render itself is the editor's, shared, not forked: js/render.js
// (renderProgram: one pass offline, faster than real time, on the engine live mode plays), with render-client.js (the
// Worker, the WAV in memory, its playback) and render-model.js beside it, all copied from site/web/js by build.mjs.

export * from './js/render.js';
export { renderOnce, playable, RENDER_RATE } from './js/render-client.js';
