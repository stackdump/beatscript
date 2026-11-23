// wasm/btw.js: the browser module under the names it had before the rename (btw -> beatscript), for one release.
// (rename-to-beatscript: keep. This file is the old names on purpose.)
//
// A tab opened before the deploy keeps its old scripts and, the first time it needs the module in a new realm (a worker,
// the composer on the page thread, a docs page's highlighter), imports ../wasm/btw.js. This answers that import with the
// new module (./beatscript.js, the same build) under the old export names: btw_check is beatscript_check, BtwRun is
// BeatscriptRun, BtwLs is BeatscriptLs, ...; the engine's own exports (Dj, DjComposer, engine_version, ...) never said btw
// and are the same. scripts/build-web-wasm.sh copies it beside the module (and into hashes.json).
//
// The old page passed init the URL of btw_bg.wasm, which a deploy may no longer serve, and old glue and a new module cannot
// be mixed: init loads ./beatscript_bg.wasm in place of an old URL or an old module. initSync cannot fetch, so given a module
// of the old build it says so (the old page's callers catch it and fall back) rather than fail inside the glue.
//
// Remove it, and its line in scripts/build-web-wasm.sh, once a release has gone by.

import init, { initSync as initSyncNew } from './beatscript.js';

export * from './beatscript.js';
export {
  beatscript_check as btw_check,
  beatscript_fmt as btw_fmt,
  beatscript_expand as btw_expand,
  beatscript_generate as btw_generate,
  beatscript_know as btw_know,
  beatscript_instruments as btw_instruments,
  beatscript_open as btw_open,
  beatscript_share as btw_share,
  beatscript_generation as btw_generation,
  beatscript_highlight as btw_highlight,
  beatscript_cheatsheet as btw_cheatsheet,
  beatscript_build_info as btw_build_info,
  BeatscriptRun as BtwRun,
  BeatscriptLs as BtwLs,
} from './beatscript.js';

const fresh = () => new URL('./beatscript_bg.wasm', import.meta.url);

/** A module compiled from the old build: it exports btw_check, not beatscript_check. */
const oldModule = (m) => typeof WebAssembly !== 'undefined' && m instanceof WebAssembly.Module && WebAssembly.Module.exports(m).some((e) => e.name === 'btw_check');

/** A URL, string or Request naming the old file. */
function oldPath(p) {
  try {
    const href = typeof Request !== 'undefined' && p instanceof Request ? p.url : p instanceof URL || typeof p === 'string' ? String(p) : null;
    return href != null && /(?:^|\/)btw_bg\.wasm$/.test(new URL(href, import.meta.url).pathname);
  } catch {
    return false;
  }
}

/** The glue's own argument shape: `{module_or_path}` (or `{module}`), else the deprecated bare argument. */
const unwrap = (arg, key) => (arg != null && Object.getPrototypeOf(arg) === Object.prototype ? arg[key] : arg);

export default async function initLegacy(arg) {
  let m = unwrap(arg, 'module_or_path');
  if (m == null || oldPath(m) || oldModule(m)) m = fresh();
  else if (ArrayBuffer.isView(m) || m instanceof ArrayBuffer) {
    const compiled = await WebAssembly.compile(m);
    m = oldModule(compiled) ? fresh() : compiled;
  }
  return init({ module_or_path: m });
}

export function initSync(arg) {
  let m = unwrap(arg, 'module');
  if (m != null && (ArrayBuffer.isView(m) || m instanceof ArrayBuffer)) m = new WebAssembly.Module(m);
  if (oldModule(m)) throw new Error('wasm/btw.js: this module is from the build before the rename, and this page has the new one: reload the page');
  return initSyncNew({ module: m });
}
