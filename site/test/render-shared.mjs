// The shared render module (site/web/js/render.js, render-client.js, render-model.js) in Node, on the real engine:
// the editor's render source and the player's mode="render" play the same code, not a fork of it.
//
//   BEATSCRIPT_WASM=<dir> node site/test/render-shared.mjs     (needs beatscript.js + beatscript_bg.wasm: site/web/wasm by default; skips without)
//
// Checks:
//   shared     the player package (build.mjs) carries the editor's render files byte for byte (js/render.js and its
//              client, worker and model), and its beatscript-render.js / render-model.js only re-export them
//   same PCM   renderOnce from render-client.js (the editor's path, here on this thread) and the package's
//              renderProgram render the same program to the same 16-bit PCM (sha256), and again (deterministic)
//   player     RenderPlayer on that real render: one pass of the program's length, a clock that is the audio position,
//              the CID cache serving a second play without a render, Stop disposing the playback

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.join(here, '..', 'web');
const wasm = process.env.BEATSCRIPT_WASM || path.join(web, 'wasm');
if (!fs.existsSync(path.join(wasm, 'beatscript_bg.wasm'))) {
  console.log(`# skip: no ${wasm}/beatscript_bg.wasm (scripts/build-web-wasm.sh ${wasm})`);
  process.exit(0);
}

const failures = [];
const check = (ok, what) => { console.log(`${ok ? 'ok' : 'FAIL'} - ${what}`); if (!ok) failures.push(what); };
const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');

// the editor's js/ with the module beside it, as the site serves them (render.js imports ../wasm/beatscript.js)
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'btw-render-shared-'));
fs.cpSync(path.join(web, 'js'), path.join(root, 'js'), { recursive: true });
fs.mkdirSync(path.join(root, 'wasm'));
for (const f of ['beatscript.js', 'beatscript_bg.wasm']) fs.copyFileSync(path.join(wasm, f), path.join(root, 'wasm', f));
const dist = path.join(root, 'player');
execFileSync(process.execPath, [path.join(here, '..', 'player', 'build.mjs'), '--wasm', wasm, '--out', dist], { stdio: 'inherit' });

// ---- shared, not forked
const SHARED = ['render.js', 'render-client.js', 'render-worker.js', 'render-model.js'];
for (const f of SHARED) {
  check(fs.existsSync(path.join(dist, 'js', f)) && sha(fs.readFileSync(path.join(dist, 'js', f))) === sha(fs.readFileSync(path.join(web, 'js', f))), `shared: the package's js/${f} is the editor's, byte for byte`);
}
for (const f of ['beatscript-render.js', 'render-model.js']) {
  const code = fs.readFileSync(path.join(dist, f), 'utf8').replace(/^\s*\/\/.*$/gm, '').trim();
  check(/^(export \* from '\.\/js\/render(-model)?\.js';\s*)(export \{[^}]*\} from '\.\/js\/render-client\.js';)?$/.test(code), `shared: the package's ${f} only re-exports js/ (${JSON.stringify(code)})`);
}
check(!fs.existsSync(path.join(dist, 'render-worker.js')), 'shared: no second render-worker.js at the package top');

// ---- the same PCM by both doors
const TEXT = `tempo 128
key am
let four = x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~
section verse 2 bars {
  deck A: kick four | hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~
  deck B: melody a4 ~ c5 ~ e5 ~ c5 ~ a4 ~ g4 ~ a4 ~ ~ ~
}
section chorus 2 bars {
  deck A: kick four | snare ~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~
}
arrange { verse chorus } loop
`;
const module = await WebAssembly.compile(fs.readFileSync(path.join(wasm, 'beatscript_bg.wasm')));
const client = await import(pathToFileURL(path.join(root, 'js', 'render-client.js')).href);
const model = await import(pathToFileURL(path.join(root, 'js', 'render-model.js')).href);
const pkg = await import(pathToFileURL(path.join(dist, 'beatscript-render.js')).href);
const pcmSha = (r) => sha(Buffer.from(model.pcm16(r.left, r.right).buffer));

const progress = [];
const a = await client.renderOnce(module, TEXT, { onProgress: (p) => progress.push(p.fraction) });
check(a.via === 'page', `the editor's path renders here on this thread when there is no Worker (${a.via})`);
const frames = model.framesOf(4, 128, 44100);
check(a.result.frames === frames && a.result.bars === 4, `one pass of the looping song: 4 bars at 128, ${frames} frames (${a.result.frames})`);
check(a.result.loops === true, 'the render knows the song loops (the editor loops its audio)');
const b = await pkg.renderProgram({ module, text: TEXT, sampleRate: 44100 });
const c = await client.renderOnce(module, TEXT, {});
check(pcmSha(a.result) === pcmSha(b), `the editor's render and the player's are the same PCM (${pcmSha(a.result).slice(0, 16)})`);
check(pcmSha(a.result) === pcmSha(c.result), 'and the same again: the render is deterministic');
check(progress.length > 0 && progress.at(-1) === 1, `progress is reported up to 100% (${progress.length} reports)`);
let rms = 0;
for (let i = 0; i < a.result.frames; i++) rms += a.result.left[i] * a.result.left[i];
rms = Math.sqrt(rms / a.result.frames);
check(rms > 0.01, `it is not silent (RMS ${rms.toFixed(4)})`);

// an aborted render stops with an AbortError
const ac = new AbortController();
const going = client.renderOnce(module, TEXT, { signal: ac.signal });
ac.abort();
let abortName = null;
try { await going; } catch (e) { abortName = e.name; }
check(abortName === 'AbortError', `a stopped render rejects with an AbortError (${abortName})`);

// ---- RenderPlayer on the real render (Web Audio transport on a clock set by hand)
const { RenderPlayer, RenderCache } = await import(pathToFileURL(path.join(root, 'js', 'render-player.js')).href);
const glue = await import(pathToFileURL(path.join(root, 'wasm', 'beatscript.js')).href);
glue.initSync({ module });
const report = JSON.parse(glue.beatscript_check(TEXT));
const ctx = {
  currentTime: 0, sampleRate: 44100, destination: {}, resume: async () => {},
  createBuffer(ch, n, sr) { return { length: n, sampleRate: sr, duration: n / sr, copyToChannel() {} }; },
  createBufferSource() { const s = { connect() {}, disconnect() {}, start() { s.started = true; }, stop() { s.stopped = true; } }; return s; },
};
let renders = 0;
const counted = (...args) => { renders++; return client.renderOnce(...args); };
const cache = new RenderCache();
const p = new RenderPlayer({ context: ctx, destination: {}, module, text: TEXT, report, cache, render: counted, element: false });
await p.start();
check(p.state === 'playing' && Math.abs(p.duration - frames / 44100) < 1e-9, `the player plays the render: ${p.duration.toFixed(3)} s`);
check(p.loop === true, 'a looping song loops its render');
p.echo([{ op: 'play', deck: 'A', on: true }]);
ctx.currentTime = 1.875; // one bar at 128
check(Math.abs(p.grid().beat - 4) < 1e-9, `the clock is the audio position: one bar in is beat 4 (${p.grid().beat})`);
p.stop();
check(p.state === 'stopped' && p.transport === null, 'Stop disposes the playback');
const q = new RenderPlayer({ context: ctx, destination: {}, module, text: TEXT, report, cache, render: counted, element: false });
await q.start();
check(renders === 1 && q.rendered.cached === true, `the same CID plays from the cache: ${renders} render`);
q.stop();

fs.rmSync(root, { recursive: true, force: true });
console.log(failures.length ? `# ${failures.length} failed` : '# all passed');
process.exit(failures.length ? 1 : 0);
