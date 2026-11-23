// build.mjs: assemble the studio package, @stackdump/beatscript-studio, into one directory, publish-ready.
//
//   node site/studio/build.mjs [--wasm <dir>] [--out <dir>]
//
// --wasm: the browser module (btw.js + btw_bg.wasm), default site/web/wasm. --out: default site/studio/dist
// (git-ignored, as the player's dist is: the wasm is built from a private repo, so the package is assembled, never
// committed).
//
// The studio carries the whole player: the player's build runs first into the same directory, so there is one engine,
// one module and one copy of every file on a page that has both elements. Then the studio adds:
//   beatscript-studio.js     the element             tool-surface.js        its tools, MCP-shaped (./tools)
//   studio-css.js            its stylesheet          beatscript-studio.d.ts its types
//   js/editor.js and what it imports, from site/web/js, unchanged (geometry.js, wrap-model.js, highlight.js, util.js)
//   test/csp.html            a page under an artifact-like CSP (the player's own page is test/player-csp.html)
//   package.json, README.md, hashes.json (sha256 of every file, and the module's own hashes.json)
// embed-config.js (./config) is the player's: its build copies it. No npm, no bundler: files are copied as they are.

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.join(here, '..', 'web');
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const wasmDir = path.resolve(arg('--wasm', path.join(web, 'wasm')));
const out = path.resolve(arg('--out', path.join(here, 'dist')));

const VERSION = '0.1.0';
/** The studio's own files, at the package's top level. */
const OWN = ['beatscript-studio.js', 'beatscript-studio.d.ts', 'tool-surface.js', 'studio-css.js'];
/** The editor's modules the element uses; what they import comes with them. */
const EDITOR = ['editor.js', 'geometry.js', 'wrap-model.js', 'highlight.js', 'util.js'];

execFileSync(process.execPath, [path.join(here, '..', 'player', 'build.mjs'), '--wasm', wasmDir, '--out', out], { stdio: 'inherit' });

const cp = (from, to) => fs.copyFileSync(from, path.join(out, to));
for (const f of OWN) cp(path.join(here, 'src', f), f);
cp(path.join(here, 'README.md'), 'README.md');
fs.renameSync(path.join(out, 'test', 'csp.html'), path.join(out, 'test', 'player-csp.html'));
cp(path.join(here, 'test', 'csp.html'), 'test/csp.html');

/** A file's relative imports, read from its code without its comments (a doc comment's example import is not one). */
const importsOf = (file) => {
  const src = fs.readFileSync(file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  return { src, specs: [...src.matchAll(/(?:from\s+|import\s*\(\s*|import\s+)(['"])(\.[^'"]+)\1/g)].map((m) => m[2]) };
};

// the editor's modules and everything they (and the studio's own files) import from js/, copied from site/web/js
const want = [...EDITOR];
for (const f of OWN.filter((f) => f.endsWith('.js'))) {
  for (const s of importsOf(path.join(out, f)).specs) if (s.startsWith('./js/')) want.push(s.slice(5));
}
const seen = new Set();
while (want.length) {
  const f = want.pop();
  if (seen.has(f)) continue;
  seen.add(f);
  const from = path.join(web, 'js', f);
  if (!fs.existsSync(from)) continue; // reported below as an import the package does not have
  if (!fs.existsSync(path.join(out, 'js', f))) cp(from, `js/${f}`);
  for (const s of importsOf(from).specs) if (/^\.\/[^/]+$/.test(s)) want.push(s.slice(2));
}

// the player's two checks, over every .js file in the package (every directory): every import is a file inside it,
// and beatscript.app is only ever a plain link (nothing is fetched from it)
const jsFiles = (d) => fs.readdirSync(path.join(out, d)).flatMap((f) => {
  const rel = path.join(d, f);
  return fs.statSync(path.join(out, rel)).isDirectory() ? jsFiles(rel) : f.endsWith('.js') ? [rel] : [];
});
let scanned = 0;
for (const rel of jsFiles('')) {
  const { src, specs } = importsOf(path.join(out, rel));
  for (const s of specs) {
    const to = path.resolve(out, path.dirname(rel), s);
    if (!to.startsWith(out + path.sep) || !fs.existsSync(to)) throw new Error(`${rel} imports ${s}, which the package does not have`);
  }
  // (the embed config's schema $id is an identifier, never fetched)
  if (/https?:\/\/beatscript\.app\/(?!p\/|docs|#|embed-config\.schema\.json)/.test(src)) throw new Error(`${rel} names a beatscript.app URL that is not a plain link`);
  scanned++;
}

// package.json: the studio's, over the player's; it depends on nothing (the player is inside it)
const top = fs.readdirSync(out).filter((f) => f !== 'hashes.json' && f !== 'package.json').sort();
const pkg = {
  name: '@stackdump/beatscript-studio',
  version: VERSION,
  description: '<beatscript-studio>: the BeatScript editor, its checker and the player in one self-contained element, and its tools as an MCP-shaped surface for a Claude artifact or any page. The engine (WebAssembly, inlined) and the whole <beatscript-track> player are bundled; nothing is fetched from beatscript.app.',
  type: 'module',
  module: './beatscript-studio.js',
  browser: './beatscript-studio.js',
  types: './beatscript-studio.d.ts',
  exports: {
    '.': { types: './beatscript-studio.d.ts', import: './beatscript-studio.js' },
    './tools': './tool-surface.js',
    './config': './embed-config.js',
    './player': { types: './beatscript-track.d.ts', import: './beatscript-track.js' },
    './package.json': './package.json',
  },
  jsdelivr: './beatscript-studio.js',
  unpkg: './beatscript-studio.js',
  files: [...top.map((f) => (fs.statSync(path.join(out, f)).isDirectory() ? `${f}/` : f)), 'hashes.json'],
  sideEffects: true,
  keywords: ['beatscript', 'music', 'live-coding', 'web-component', 'webassembly', 'editor', 'mcp', 'claude-artifact'],
  homepage: 'https://beatscript.app/docs/embed',
  repository: { type: 'git', url: 'git+https://github.com/stackdump/beatscript.git', directory: 'site/studio' },
  bugs: 'https://github.com/stackdump/beatscript/issues',
  author: 'Matt York (stackdump)',
  license: 'MIT AND CC0-1.0',
  publishConfig: { access: 'public' },
};
for (const [k, v] of Object.entries(pkg.exports)) {
  for (const p of typeof v === 'string' ? [v] : Object.values(v)) if (!fs.existsSync(path.join(out, p))) throw new Error(`package.json exports ${k} as ${p}, which the package does not have`);
}
fs.writeFileSync(path.join(out, 'package.json'), JSON.stringify(pkg, null, 2) + '\n');

// hashes.json again, now over every file of the studio's package
const files = {};
const walk = (d) => {
  for (const f of fs.readdirSync(path.join(out, d)).sort()) {
    const rel = path.join(d, f);
    if (rel === 'hashes.json') continue;
    const st = fs.statSync(path.join(out, rel));
    if (st.isDirectory()) walk(rel);
    else files[rel] = { sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(out, rel))).digest('hex'), size: st.size };
  }
};
walk('');
let module = null;
try { module = JSON.parse(fs.readFileSync(path.join(wasmDir, 'hashes.json'), 'utf8')); } catch { /* an older build */ }
fs.writeFileSync(path.join(out, 'hashes.json'), JSON.stringify({ files, module }, null, 2) + '\n');
const total = Object.values(files).reduce((a, f) => a + f.size, 0);
console.log(`build.mjs: wrote the studio ${VERSION} to ${out}: ${Object.keys(files).length} files, ${(total / 1048576).toFixed(1)} MB (${scanned} modules checked: imports, links)`);
