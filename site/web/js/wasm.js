// The module facade. Loads the real wasm (wasm/beatscript.js + beatscript_bg.wasm, built from the Rust crates and
// committed to the deploy by hand, never to git) or, with ?mock=1, the stand-in in wasm-mock.js, and
// answers parsed JSON. If the module cannot be loaded the page degrades: `loadBeatscript` rejects and the page
// says so and keeps a plain editor.
//
// Every export is looked up by its name (beatscript_check, BeatscriptRun, ...) and, for the release after the rename, by the
// name it had before (btw_check, BtwRun, ...: LEGACY), so a module from either side of the deploy works. A required export
// that is in neither spelling is an error here, at load, never a feature that quietly turns off; an optional one is null and
// the page has a fallback for it (FEATURES says which). (rename-to-beatscript: keep. This file spells the old names on purpose.)

const j = (s) => (typeof s === 'string' ? JSON.parse(s) : s);

/** The module's exports the page uses: facade name -> [export, required]. An optional one's facade member is null when it is missing. */
export const FEATURES = Object.freeze({
  check: ['beatscript_check', true],
  fmt: ['beatscript_fmt', true],
  open: ['beatscript_open', true],
  share: ['beatscript_share', true],
  instruments: ['beatscript_instruments', true],
  generation: ['beatscript_generation', true],
  cheatsheet: ['beatscript_cheatsheet', true],
  highlight: ['beatscript_highlight', true],
  newRun: ['BeatscriptRun', true],
  /** an `import` of a CID the page was told; without it the page cannot check a program that imports one */
  know: ['beatscript_know', false],
  /** the seeded generator; without it the page asks POST /api/generate */
  generate: ['beatscript_generate', false],
  /** a radio station's answer; djweb does not have it (the page asks POST /api/station) */
  station: ['beatscript_station', false],
  /** the language service (crates/beatscriptls); without it the editor has no completion, hover or quick-fixes */
  ls: ['BeatscriptLs', false],
});

/** Each export's name before the rename (btw -> beatscript): read when the new name is not there. */
export const LEGACY = Object.freeze({
  beatscript_check: 'btw_check',
  beatscript_fmt: 'btw_fmt',
  beatscript_expand: 'btw_expand',
  beatscript_generate: 'btw_generate',
  beatscript_know: 'btw_know',
  beatscript_instruments: 'btw_instruments',
  beatscript_open: 'btw_open',
  beatscript_share: 'btw_share',
  beatscript_generation: 'btw_generation',
  beatscript_highlight: 'btw_highlight',
  beatscript_cheatsheet: 'btw_cheatsheet',
  beatscript_build_info: 'btw_build_info',
  beatscript_station: 'btw_station',
  BeatscriptRun: 'BtwRun',
  BeatscriptLs: 'BtwLs',
});

/** The export `name` of `mod`, under its name or the one it had before the rename; null when it has neither. */
export function exportOf(mod, name) {
  if (typeof mod[name] === 'function') return mod[name];
  const old = LEGACY[name];
  return old && typeof mod[old] === 'function' ? mod[old] : null;
}

/** Which of FEATURES the module has: {found: [...], missing: [...], missingRequired: [...]}. */
export function featuresOf(mod) {
  const found = [];
  const missing = [];
  const missingRequired = [];
  for (const [feature, [name, required]] of Object.entries(FEATURES)) {
    if (exportOf(mod, name)) found.push(feature);
    else {
      missing.push(feature);
      if (required) missingRequired.push(`${feature} (${name})`);
    }
  }
  return { found, missing, missingRequired };
}

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
  const { missingRequired } = featuresOf(mod);
  if (missingRequired.length) throw new Error(`the BeatScript module lacks ${missingRequired.join(', ')}: it is not the one this page was built with`);
  const f = (name) => exportOf(mod, name);
  const check = f('beatscript_check');
  const fmt = f('beatscript_fmt');
  const open = f('beatscript_open');
  const share = f('beatscript_share');
  const know = f('beatscript_know');
  const instruments = f('beatscript_instruments');
  const generation = f('beatscript_generation');
  const cheatsheet = f('beatscript_cheatsheet');
  const expand = f('beatscript_expand');
  const generate = f('beatscript_generate');
  const station = f('beatscript_station');
  const highlight = f('beatscript_highlight');
  const Run = f('BeatscriptRun');
  const Ls = f('BeatscriptLs');
  let lastText = null;
  let lastSpans = [];
  return {
    mock: !!mod.mock,
    wasmUrl,
    check: (text) => j(check(text)),
    fmt: (text) => j(fmt(text)),
    open: (s) => j(open(s)),
    share: (text, sig = null, notes = null) => j(share(text, sig, notes)),
    /** Tell the module a program by CID and canonical text (what the server answers for the CID, or a link carried): an `import` of it then checks. null when the module predates it. */
    know: know ? (cid, text) => j(know(cid, text)) : null,
    instruments: () => j(instruments()),
    generation: (text, scope, n) => j(generation(text, scope || null, n)),
    cheatsheet: () => cheatsheet(),
    /** The program with its seed(...), template(...) and gen@N references written out as plain BeatScript; lineage opens it with `#! forked-from:`. null when the module predates it (and in the mock). */
    expand: expand ? (text, lineage = false) => j(expand(text, !!lineage)) : null,
    /** The seeded generator ({genre,bpm,key,length,seed} -> {ok,text,cid,check,...}); null when the module predates it. */
    generate: generate ? (req) => j(generate(JSON.stringify(req || {}))) : null,
    /** A radio station's answer (station-model.js: {station, n, count, segue, ...} -> {ok, station, tracks, segues, ...}); null when the module does not have it (the page asks POST /api/station). */
    station: station ? (req) => j(station(JSON.stringify(req || {}))) : null,
    /** [[start16, end16, class], ...] */
    highlight(text) {
      if (text === lastText) return lastSpans;
      lastSpans = j(highlight(text));
      lastText = text;
      return lastSpans;
    },
    newRun() {
      return wrapRun(new Run());
    },
    /** The language service (BeatscriptLs, crates/beatscriptls) when the module has it; null in the mock or an older module. */
    ls: Ls ? () => new Ls() : null,
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
