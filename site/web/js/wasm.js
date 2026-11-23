// The module facade. Loads the real wasm (wasm/beatscript.js + beatscript_bg.wasm, built from the Rust crates and
// committed to the deploy by hand, never to git) or, with ?mock=1, the stand-in in wasm-mock.js, and
// answers parsed JSON. If the module cannot be loaded the page degrades: `loadBeatscript` rejects and the page
// says so and keeps a plain editor.

const j = (s) => (typeof s === 'string' ? JSON.parse(s) : s);

export async function loadBeatscript({ mock = false } = {}) {
  let mod;
  let wasmUrl = null;
  if (mock) {
    mod = (await import('./wasm-mock.js')).mockModule();
  } else {
    const glue = new URL('../wasm/beatscript.js', import.meta.url);
    wasmUrl = new URL('../wasm/beatscript_bg.wasm', import.meta.url).href;
    mod = await import(glue.href);
    await mod.default({ module_or_path: wasmUrl });
  }
  return makeFacade(mod, wasmUrl);
}

export function makeFacade(mod, wasmUrl = null) {
  let lastText = null;
  let lastSpans = [];
  return {
    mock: !!mod.mock,
    wasmUrl,
    check: (text) => j(mod.beatscript_check(text)),
    fmt: (text) => j(mod.beatscript_fmt(text)),
    open: (s) => j(mod.beatscript_open(s)),
    share: (text, sig = null, notes = null) => j(mod.beatscript_share(text, sig, notes)),
    /** Tell the module a program by CID and canonical text (what the server answers for the CID, or a link carried): an `import` of it then checks. null when the module predates it. */
    know: typeof mod.beatscript_know === 'function' ? (cid, text) => j(mod.beatscript_know(cid, text)) : null,
    instruments: () => j(mod.beatscript_instruments()),
    generation: (text, scope, n) => j(mod.beatscript_generation(text, scope || null, n)),
    cheatsheet: () => mod.beatscript_cheatsheet(),
    /** The program with its seed(...), template(...) and gen@N references written out as plain BeatScript; lineage opens it with `#! forked-from:`. null when the module predates it (and in the mock). */
    expand: typeof mod.beatscript_expand === 'function' ? (text, lineage = false) => j(mod.beatscript_expand(text, !!lineage)) : null,
    /** The seeded generator ({genre,bpm,key,length,seed} -> {ok,text,cid,check,...}); null when the module predates it. */
    generate: mod.beatscript_generate ? (req) => j(mod.beatscript_generate(JSON.stringify(req || {}))) : null,
    /** A radio station's answer (station-model.js: {station, n, count, segue, ...} -> {ok, station, tracks, segues, ...}); null when the module predates it (the page asks POST /api/station). */
    station: typeof mod.beatscript_station === 'function' ? (req) => j(mod.beatscript_station(JSON.stringify(req || {}))) : null,
    /** [[start16, end16, class], ...] */
    highlight(text) {
      if (text === lastText) return lastSpans;
      lastSpans = j(mod.beatscript_highlight(text));
      lastText = text;
      return lastSpans;
    },
    newRun() {
      return wrapRun(new mod.BeatscriptRun());
    },
    /** The language service (BeatscriptLs, crates/beatscriptls) when the module has it; null in the mock or an older module. */
    ls: typeof mod.BeatscriptLs === 'function' ? () => new mod.BeatscriptLs() : null,
  };
}

function wrapRun(raw) {
  return {
    raw,
    eval: (text) => j(raw.eval(text)),
    stop: () => j(raw.stop()) || [],
    /** grid: {deck, beat, bpm, playing:{A,B}} or null */
    service: (grid) => j(raw.service(JSON.stringify(grid || null))) || [],
    marks: () => j(raw.marks()),
    status: () => j(raw.status()),
    cid: () => raw.running_cid() || null,
    free() {
      if (raw.free) raw.free();
    },
  };
}
